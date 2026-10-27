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

pub(crate) struct State {
    token: String,
    identity_key: String,
    pub(crate) host: String,
    upstream: Arc<Upstream>,
    cache: Mutex<HashMap<(String, String, u64), Slot>>,
    graphql: Mutex<HashMap<Vec<u8>, GraphqlSlot>>,
    graphql_fresh: AtomicU64,
    reads: AtomicU64,
    fresh: AtomicU64,
    started: Instant,
    pub(crate) fresh_for: Duration,
    pub(crate) journal: ghd_core::coherence::Journal,
    cache_epoch: Mutex<u64>,
    pub(crate) github_token: String,
    pub(crate) gh_reads: crate::gh_reads::Cache,
    pub(crate) rest_reads: crate::rest_reads::Cache,
    pub(crate) rest_counters: crate::rest_reads::Counters,
    pub(crate) gh_permits: kernal_api::async_engine::Semaphore,
    pub(crate) render_root: std::path::PathBuf,
}

pub(crate) fn new_state(
    identity: &ghd_core::identity::Identity,
    token: String,
) -> io::Result<State> {
    Ok(State {
        token,
        identity_key: identity.key().to_string(),
        host: identity.host().to_string(),
        upstream: Arc::new(Upstream::new(identity, BATCH_WINDOW)?),
        cache: Mutex::new(HashMap::new()),
        graphql: Mutex::new(HashMap::new()),
        graphql_fresh: AtomicU64::new(0),
        reads: AtomicU64::new(0),
        fresh: AtomicU64::new(0),
        started: Instant::now(),
        fresh_for: fresh_for(),
        journal: ghd_core::coherence::Journal::global()?,
        cache_epoch: Mutex::new(0),
        github_token: identity.token().to_string(),
        gh_reads: Mutex::new(HashMap::new()),
        rest_reads: Mutex::new(HashMap::new()),
        rest_counters: crate::rest_reads::Counters::default(),
        gh_permits: kernal_api::async_engine::Semaphore::new(4),
        render_root: endpoint::directory(identity)?.join("renders"),
    })
}

async fn sync_epoch(state: &State, epoch: u64) -> io::Result<()> {
    let mut current = state.cache_epoch.lock().await;
    if *current > epoch {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "GitHub changed while starting the read; retry",
        ));
    }
    if *current != epoch {
        state.cache.lock().await.clear();
        state.graphql.lock().await.clear();
        state.gh_reads.lock().await.clear();
        state.rest_reads.lock().await.clear();
        *current = epoch;
    }
    Ok(())
}

pub(crate) async fn read_epoch(state: &State) -> io::Result<u64> {
    let epoch = state.journal.epoch()?;
    sync_epoch(state, epoch).await?;
    Ok(epoch)
}

pub fn run() -> io::Result<()> {
    let identity = ghd_core::identity::Identity::resolve(None)?;
    let runtime = RuntimeBuilder::multi_thread().enable_all().build()?;
    runtime.run(async move {
        let server = Server::bind(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            http_server::Limits::default(),
        )
        .await?;
        let port = server.local_addr()?.port();
        let token = new_token().await?;
        let state = Arc::new(new_state(&identity, token.clone())?);
        endpoint::write_for(&identity, &Endpoint { port, token })?;
        eprintln!("ghd daemon listening on 127.0.0.1:{port}");
        server
            .serve(move |request| {
                let state = Arc::clone(&state);
                async move { route(&state, request).await }
            })
            .await
    })
}

pub(crate) async fn new_token() -> io::Result<String> {
    let random = SecureRandom::new(1, Duration::from_secs(10))
        .map_err(|e| io::Error::other(e.to_string()))?;
    let bytes = random
        .bytes(16)
        .await
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

async fn route(state: &State, request: Request) -> Response {
    let authorized = request
        .header("authorization")
        .is_some_and(|v| v == format!("Bearer {}", state.token).as_bytes());
    let same_identity = request
        .header("x-ghd-identity")
        .is_some_and(|value| value == state.identity_key.as_bytes());
    if !authorized || !same_identity {
        return text(401, "unauthorized");
    }
    match (request.method(), request.path()) {
        ("GET", "/v1/health") => text(200, "ok"),
        ("GET", "/v1/stats") => stats(state),
        ("POST", "/v1/pr/view") => pr_view(state, request.body()).await,
        ("POST", "/v1/graphql") => graphql(state, request.body()).await,
        ("POST", "/v1/read/gh") => crate::gh_reads::handle(state, request.body()).await,
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
    if !req.hostname.is_empty()
        && !ghd_core::identity::hostname(&req.hostname).is_ok_and(|host| host == state.host)
    {
        return failure(400, "request host does not match daemon identity".into());
    }
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
    if wanted.is_empty() {
        return failure(400, "at least one JSON field is required".into());
    }
    let epoch = match read_epoch(state).await {
        Ok(epoch) => epoch,
        Err(error) => return failure(409, error.to_string()),
    };
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
    let result = read(state, &req, &wanted, &mut entry).await;
    if let Err(error) = state.journal.confirm(epoch) {
        return failure(409, error.to_string());
    }
    match result {
        Ok((fields, source)) => {
            let doc: BTreeMap<String, Value> = wanted
                .iter()
                .filter_map(|f| fields.get(f).map(|v| (f.clone(), v.clone())))
                .collect();
            match json::encode(&Value::Object(doc), Layout::Pretty) {
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
        .filter(|e| covers(e) && e.validated.elapsed() < state.fresh_for)
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

fn graphql_http_answer(status: u16, json: Vec<u8>, source: &str) -> Response {
    if status >= 400 {
        return graphql_reply(
            200,
            &GraphqlResponse {
                json,
                source: source.into(),
                error: format!("GitHub returned HTTP {status}"),
            },
        );
    }
    graphql_answer(json, source)
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
    if !req.hostname.is_empty()
        && !ghd_core::identity::hostname(&req.hostname).is_ok_and(|host| host == state.host)
    {
        return failed(400, "request host does not match daemon identity".into());
    }
    let upstream_body = match gql::body_operation(&req.query, &req.variables, &req.operation_name) {
        Ok(b) => b,
        Err(e) => return failed(400, e),
    };
    let operation = match gql::classify(
        &req.query,
        (!req.operation_name.is_empty()).then_some(req.operation_name.as_str()),
    ) {
        Ok(operation) => operation,
        Err(error) => return failed(400, error.to_string()),
    };
    if operation == gql::Operation::Write {
        let intent = match state.journal.begin() {
            Ok(intent) => intent,
            Err(error) => return failed(409, error.to_string()),
        };
        state.cache.lock().await.clear();
        state.graphql.lock().await.clear();
        state.gh_reads.lock().await.clear();
        state.rest_reads.lock().await.clear();
        return match state.upstream.graphql_raw(&upstream_body).await {
            Ok((status, json)) => {
                if status < 500
                    && let Err(error) = intent.finish()
                {
                    return failed(500, error.to_string());
                }
                graphql_http_answer(status, json, "fetched")
            }
            // Keep the intent when upstream completion is uncertain.
            Err(error) => failed(502, error),
        };
    }
    let epoch = match read_epoch(state).await {
        Ok(epoch) => epoch,
        Err(error) => return failed(409, error.to_string()),
    };
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
        && at.elapsed() < state.fresh_for
    {
        state.graphql_fresh.fetch_add(1, Ordering::Relaxed);
        return match state.journal.confirm(epoch) {
            Ok(()) => graphql_answer(json.clone(), "fresh"),
            Err(error) => failed(409, error.to_string()),
        };
    }
    match state.upstream.graphql_raw(&upstream_body).await {
        Ok((status, json)) => {
            if let Err(error) = state.journal.confirm(epoch) {
                return failed(409, error.to_string());
            }
            // Only a clean answer is reused; errors (rate limits) are not.
            if status == 200 && gql::errors(&json).is_none() {
                *entry = Some((json.clone(), Instant::now()));
            }
            graphql_http_answer(status, json, "fetched")
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
    if let Err(error) = state.rest_counters.add_stats(&mut doc) {
        return text(500, &error.to_string());
    }
    match json::encode(&Value::Object(doc), Layout::Pretty) {
        Ok(body) => Response::new(200, body)
            .and_then(|r| r.with_header("content-type", "application/json"))
            .unwrap_or_default(),
        Err(e) => text(500, &e.to_string()),
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod identity_tests;

#[cfg(test)]
#[path = "coherence_tests.rs"]
mod coherence_tests;
