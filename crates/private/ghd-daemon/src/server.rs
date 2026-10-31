//! The daemon: loopback HTTP routes over a per-PR cache.
//!
//! A read is answered, cheapest first, from:
//! 1. `fresh`: validated within the fresh window (2 s, `GHD_FRESH_MS`), no upstream call at all;
//! 2. `revalidated`: the PR's REST ETag returned 304, which costs no budget;
//! 3. `fetched`: the batched GraphQL reader.
//!
//! Each PR has its own async lock, so concurrent identical reads make one
//! upstream round trip and the rest are served `fresh` behind it.

use crate::graphql as gql;
use crate::upstream::{Fields, Upstream};
use ghd_core::endpoint::{self, Endpoint};
use ghd_core::fields;
use ghd_core::proto::{GraphqlRequest, GraphqlResponse, PrViewRequest, PrViewResponse};
use kernal_api::async_engine::{Mutex, RuntimeBuilder};
use kernal_api::http_server::{self, Request, Response, Server};
use kernal_api::json::{self, Layout, Value};
use kernal_api::random::SecureRandom;
use prost::Message;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const DEFAULT_FRESH_FOR: Duration = Duration::from_secs(2);

/// How long a validated entry is served without asking GitHub at all:
/// `GHD_FRESH_MS` when set (read once at startup), else 2 s.
fn fresh_for() -> Duration {
    static FRESH: OnceLock<Duration> = OnceLock::new();
    *FRESH.get_or_init(|| {
        std::env::var("GHD_FRESH_MS")
            .ok()
            .and_then(|ms| ms.parse().ok())
            .map_or(DEFAULT_FRESH_FOR, Duration::from_millis)
    })
}
const BATCH_WINDOW: Duration = Duration::from_millis(50);

struct Entry {
    etag: Option<String>,
    fields: Fields,
    validated: Instant,
}

type Slot = Arc<Mutex<Option<Entry>>>;

/// A cached `api graphql` answer: the response body and when it was fetched.
type GraphqlSlot = Arc<Mutex<Option<(Vec<u8>, Instant)>>>;

/// Past this many cached queries, stale ones are dropped on insert.
const GRAPHQL_CACHE_MAX: usize = 4096;

struct State {
    token: String,
    upstream: Arc<Upstream>,
    cache: Mutex<HashMap<(String, String, u64), Slot>>,
    graphql: Mutex<HashMap<Vec<u8>, GraphqlSlot>>,
    graphql_fresh: AtomicU64,
    reads: AtomicU64,
    fresh: AtomicU64,
    started: Instant,
}

pub fn run() -> io::Result<()> {
    let gh_token = github_token()?;
    let runtime = RuntimeBuilder::multi_thread().enable_all().build()?;
    runtime.run(async move {
        let server = Server::bind(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            http_server::Limits::default(),
        )
        .await?;
        let port = server.local_addr()?.port();
        let token = new_token().await?;
        let state = Arc::new(State {
            token: token.clone(),
            upstream: Arc::new(Upstream::new(gh_token, BATCH_WINDOW)?),
            cache: Mutex::new(HashMap::new()),
            graphql: Mutex::new(HashMap::new()),
            graphql_fresh: AtomicU64::new(0),
            reads: AtomicU64::new(0),
            fresh: AtomicU64::new(0),
            started: Instant::now(),
        });
        endpoint::write(&Endpoint { port, token })?;
        eprintln!("ghd daemon listening on 127.0.0.1:{port}");
        server
            .serve(move |request| {
                let state = Arc::clone(&state);
                async move { route(&state, request).await }
            })
            .await
    })
}

async fn new_token() -> io::Result<String> {
    let random = SecureRandom::new(1, Duration::from_secs(10))
        .map_err(|e| io::Error::other(e.to_string()))?;
    let bytes = random
        .bytes(16)
        .await
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn github_token() -> io::Result<String> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(token) = std::env::var(var)
            && !token.is_empty()
        {
            return Ok(token);
        }
    }
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()?;
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || token.is_empty() {
        return Err(io::Error::other(
            "no GitHub token: set GH_TOKEN or run `gh auth login`",
        ));
    }
    Ok(token)
}

async fn route(state: &State, request: Request) -> Response {
    let authorized = request
        .header("authorization")
        .is_some_and(|v| v == format!("Bearer {}", state.token).as_bytes());
    if !authorized {
        return text(401, "unauthorized");
    }
    match (request.method(), request.path()) {
        ("GET", "/v1/health") => text(200, &endpoint::health_body()),
        ("GET", "/v1/stats") => stats(state),
        ("POST", "/v1/pr/view") => pr_view(state, request.body()).await,
        ("POST", "/v1/graphql") => graphql(state, request.body()).await,
        _ => text(404, "no such route"),
    }
}

fn text(status: u16, body: &str) -> Response {
    Response::new(status, body.as_bytes().to_vec()).unwrap_or_default()
}

fn reply(status: u16, message: &PrViewResponse) -> Response {
    Response::new(status, message.encode_to_vec())
        .and_then(|r| r.with_header("content-type", "application/x-protobuf"))
        .unwrap_or_default()
}

fn failure(status: u16, error: String) -> Response {
    reply(
        status,
        &PrViewResponse {
            error,
            ..Default::default()
        },
    )
}

async fn pr_view(state: &State, body: &[u8]) -> Response {
    let Ok(req) = PrViewRequest::decode(body) else {
        return failure(400, "malformed request".into());
    };
    if !fields::valid_name(&req.owner) || !fields::valid_name(&req.repo) {
        return failure(400, "invalid owner/repo".into());
    }
    let mut wanted = BTreeSet::new();
    for field in &req.fields {
        if !fields::PR_FIELDS.contains(&field.as_str()) {
            return failure(
                400,
                format!(
                    "unsupported field {field:?}; supported: {}",
                    fields::PR_FIELDS.join(",")
                ),
            );
        }
        wanted.insert(field.clone());
    }
    state.reads.fetch_add(1, Ordering::Relaxed);
    let slot = {
        let mut cache = state.cache.lock().await;
        Arc::clone(
            cache
                .entry((req.owner.clone(), req.repo.clone(), req.number))
                .or_default(),
        )
    };
    let mut entry = slot.lock().await;
    match read(state, &req, &wanted, &mut entry).await {
        Ok((fields, source)) => {
            let doc: BTreeMap<String, Value> = wanted
                .iter()
                .filter_map(|f| fields.get(f).map(|v| (f.clone(), v.clone())))
                .collect();
            let layout = if req.compact {
                Layout::Compact
            } else {
                Layout::Pretty
            };
            match json::encode(&Value::Object(doc), layout) {
                Ok(json) => reply(
                    200,
                    &PrViewResponse {
                        json,
                        source: source.into(),
                        error: String::new(),
                    },
                ),
                Err(e) => failure(500, e.to_string()),
            }
        }
        Err(e) => failure(502, e),
    }
}

async fn read(
    state: &State,
    req: &PrViewRequest,
    wanted: &BTreeSet<String>,
    entry: &mut Option<Entry>,
) -> Result<(Fields, &'static str), String> {
    let covers = |e: &Entry| wanted.iter().all(|f| e.fields.contains_key(f));
    if let Some(e) = entry
        .as_ref()
        .filter(|e| covers(e) && e.validated.elapsed() < fresh_for())
    {
        state.fresh.fetch_add(1, Ordering::Relaxed);
        return Ok((e.fields.clone(), "fresh"));
    }
    // Sentinel first, then GraphQL: the fetched data is then never older than
    // the ETag it is stored under, so a later 304 cannot hide a change.
    let path = format!("/repos/{}/{}/pulls/{}", req.owner, req.repo, req.number);
    let cached_etag = entry
        .as_ref()
        .filter(|e| covers(e))
        .and_then(|e| e.etag.clone());
    let (status, etag) = state
        .upstream
        .sentinel(&path, cached_etag.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    if status == 304
        && let Some(e) = entry.as_mut()
    {
        e.validated = Instant::now();
        return Ok((e.fields.clone(), "revalidated"));
    }
    if status != 200 && status != 304 {
        return Err(format!("GitHub returned HTTP {status} for {path}"));
    }
    // Ask for everything already cached too, so the entry only ever grows.
    let mut select = wanted.clone();
    if let Some(e) = entry.as_ref() {
        select.extend(e.fields.keys().cloned());
    }
    let fields = state
        .upstream
        .pull_request(&req.owner, &req.repo, req.number, &select)
        .await?;
    *entry = Some(Entry {
        etag,
        fields: fields.clone(),
        validated: Instant::now(),
    });
    Ok((fields, "fetched"))
}

fn graphql_reply(status: u16, message: &GraphqlResponse) -> Response {
    Response::new(status, message.encode_to_vec())
        .and_then(|r| r.with_header("content-type", "application/x-protobuf"))
        .unwrap_or_default()
}

fn graphql_answer(json: Vec<u8>, source: &str) -> Response {
    let error = gql::errors(&json).unwrap_or_default();
    graphql_reply(
        200,
        &GraphqlResponse {
            json,
            source: source.into(),
            error,
        },
    )
}

async fn graphql(state: &State, body: &[u8]) -> Response {
    let failed = |status, error: String| {
        graphql_reply(
            status,
            &GraphqlResponse {
                error,
                ..Default::default()
            },
        )
    };
    let Ok(req) = GraphqlRequest::decode(body) else {
        return failed(400, "malformed request".into());
    };
    let upstream_body = match gql::body(&req.query, &req.variables) {
        Ok(b) => b,
        Err(e) => return failed(400, e),
    };
    if gql::is_mutation(&req.query) {
        // A write: never cached, and no read after it may see an older answer.
        state.graphql.lock().await.clear();
        return match state.upstream.graphql_raw(&upstream_body).await {
            Ok((_, json)) => graphql_answer(json, "fetched"),
            Err(e) => failed(502, e),
        };
    }
    let slot = {
        let mut cache = state.graphql.lock().await;
        if cache.len() >= GRAPHQL_CACHE_MAX {
            cache.retain(|_, slot| Arc::strong_count(slot) > 1);
        }
        Arc::clone(cache.entry(upstream_body.clone()).or_default())
    };
    // One lock per query: concurrent identical calls wait for the first.
    let mut entry = slot.lock().await;
    if let Some((json, at)) = entry.as_ref()
        && at.elapsed() < fresh_for()
    {
        state.graphql_fresh.fetch_add(1, Ordering::Relaxed);
        return graphql_answer(json.clone(), "fresh");
    }
    match state.upstream.graphql_raw(&upstream_body).await {
        Ok((status, json)) => {
            // Only a clean answer is reused; errors (rate limits) are not.
            if status == 200 && gql::errors(&json).is_none() {
                *entry = Some((json.clone(), Instant::now()));
            }
            graphql_answer(json, "fetched")
        }
        Err(e) => failed(502, e),
    }
}

fn stats(state: &State) -> Response {
    let c = &state.upstream.counters;
    let n = |v: &AtomicU64| Value::Unsigned(v.load(Ordering::Relaxed));
    let mut doc = BTreeMap::new();
    doc.insert(
        "uptime_secs".into(),
        Value::Unsigned(state.started.elapsed().as_secs()),
    );
    doc.insert("reads".into(), n(&state.reads));
    doc.insert("served_fresh".into(), n(&state.fresh));
    doc.insert("sentinel_304_free".into(), n(&c.sentinel_304));
    doc.insert("sentinel_200".into(), n(&c.sentinel_200));
    doc.insert("graphql_queries".into(), n(&c.graphql_queries));
    doc.insert("graphql_prs_fetched".into(), n(&c.graphql_prs));
    doc.insert("graphql_points_spent".into(), n(&c.graphql_cost));
    doc.insert("api_graphql_upstream".into(), n(&c.graphql_passthrough));
    doc.insert("api_graphql_served_fresh".into(), n(&state.graphql_fresh));
    doc.insert(
        "graphql_remaining".into(),
        Value::Signed(c.graphql_remaining.load(Ordering::Relaxed)),
    );
    doc.insert(
        "core_remaining".into(),
        Value::Signed(c.core_remaining.load(Ordering::Relaxed)),
    );
    match json::encode(&Value::Object(doc), Layout::Pretty) {
        Ok(body) => Response::new(200, body)
            .and_then(|r| r.with_header("content-type", "application/json"))
            .unwrap_or_default(),
        Err(e) => text(500, &e.to_string()),
    }
}
