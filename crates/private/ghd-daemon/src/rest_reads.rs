//! Caller-bound REST bodies, conditional validators and measured observations.

use crate::{
    gh_executor, gh_reads, gh_render, run_collection,
    server::{State, read_epoch},
};
use ghd_core::{
    proto::{GhReadRequest, GhReadResponse},
    resource::ReadPlan,
};
use kernal_api::json::Value;
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub struct Entry {
    pub(crate) body: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) etag: Option<String>,
    pub(crate) validated: Option<Instant>,
    pub(crate) ttl: Duration,
}

#[derive(Default)]
pub struct Counters {
    operations: AtomicU64,
    responses: AtomicU64,
    fresh: AtomicU64,
    revalidated: AtomicU64,
    errors: AtomicU64,
    introspections: AtomicU64,
    dropped_samples: AtomicU64,
    budgets: std::sync::Mutex<BTreeMap<String, Value>>,
}

fn header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then(|| value.trim()))
}

impl Counters {
    fn observe(&self, headers: &str) -> io::Result<()> {
        let Some(resource) = header(headers, "x-ratelimit-resource") else {
            return Ok(());
        };
        if resource.len() > 64
            || !resource
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Ok(());
        }
        let mut sample = BTreeMap::new();
        for name in ["used", "remaining", "limit", "reset"] {
            let value = header(headers, &format!("x-ratelimit-{name}"))
                .and_then(|v| v.parse::<u64>().ok())
                .map_or(Value::Null, Value::Unsigned);
            sample.insert(name.into(), value);
        }
        let mut budgets = self
            .budgets
            .lock()
            .map_err(|_| io::Error::other("REST budget lock poisoned"))?;
        if budgets.len() >= 16 && !budgets.contains_key(resource) {
            self.dropped_samples.fetch_add(1, Ordering::Relaxed);
        } else {
            budgets.insert(resource.into(), Value::Object(sample));
        }
        Ok(())
    }

    pub fn add_stats(&self, doc: &mut BTreeMap<String, Value>) -> io::Result<()> {
        for (name, value) in [
            ("rest_http_operations", &self.operations),
            ("rest_http_responses", &self.responses),
            ("rest_served_fresh", &self.fresh),
            ("rest_304", &self.revalidated),
            ("rest_errors", &self.errors),
            ("rest_rate_introspections", &self.introspections),
            ("rest_budget_samples_dropped", &self.dropped_samples),
        ] {
            doc.insert(name.into(), Value::Unsigned(value.load(Ordering::Relaxed)));
        }
        let budgets = self
            .budgets
            .lock()
            .map_err(|_| io::Error::other("REST budget lock poisoned"))?;
        doc.insert(
            "rest_provider_budgets".into(),
            Value::Object(budgets.clone()),
        );
        Ok(())
    }
}

pub(crate) struct Policy {
    pub(crate) store: bool,
    pub(crate) ttl: Duration,
    pub(crate) specified: bool,
}
pub(crate) fn policy(headers: &str, fresh: Duration) -> Policy {
    let mut out = Policy {
        store: true,
        ttl: fresh,
        specified: false,
    };
    for (name, value) in headers.lines().filter_map(|line| line.split_once(':')) {
        if name.eq_ignore_ascii_case("cache-control") {
            out.specified = true;
            for directive in value.split(',') {
                let (name, value) = directive
                    .trim()
                    .split_once('=')
                    .unwrap_or((directive.trim(), ""));
                if name.eq_ignore_ascii_case("no-store") {
                    out.store = false;
                }
                if name.eq_ignore_ascii_case("no-cache") {
                    out.ttl = Duration::ZERO;
                }
                if name.eq_ignore_ascii_case("max-age") {
                    let seconds = value.trim_matches('"').parse::<u64>().unwrap_or(0);
                    let age = header(headers, "age")
                        .and_then(|value| value.parse::<u64>().ok())
                        .unwrap_or(0);
                    out.ttl = out
                        .ttl
                        .min(Duration::from_secs(seconds.saturating_sub(age)));
                }
            }
        }
        if name.eq_ignore_ascii_case("vary") && value.split(',').any(|part| part.trim() == "*") {
            out.store = false;
        }
    }
    out
}

pub async fn run(
    state: &State,
    request: &GhReadRequest,
    plan: &ReadPlan,
) -> io::Result<GhReadResponse> {
    let rest = required_rest(plan)?;
    let epoch = read_epoch(state).await?;
    let _admission = crate::retention::admit(state).await?;
    let key = crate::retention::key("rest", gh_reads::key(request, plan).as_bytes());
    let slot = crate::retention::rest(state, &key).await;
    let mut retained = slot.lock().await;
    let mut entry = retained.take();
    if entry.is_none() {
        entry = crate::persistence::restore_rest(state, &key, epoch).await?;
    }
    if let Some(response) = fresh(state, request, plan, entry.as_ref(), epoch).await? {
        *retained = entry;
        return Ok(response);
    }
    let response = fetch(state, request, rest, entry.as_ref()).await?;
    state.journal.confirm(epoch)?;
    let (status, etag, body) = match run_collection::included_response(&response.stdout) {
        Ok(decoded) => decoded,
        Err(_) if response.exit_code != 0 => {
            state.rest_counters.errors.fetch_add(1, Ordering::Relaxed);
            return Ok(response);
        }
        Err(error) => {
            state.rest_counters.errors.fetch_add(1, Ordering::Relaxed);
            return Err(error);
        }
    };
    state
        .rest_counters
        .responses
        .fetch_add(1, Ordering::Relaxed);
    let headers = std::str::from_utf8(&response.stdout[..response.stdout.len() - body.len()])
        .map_err(io::Error::other)?;
    state.rest_counters.observe(headers)?;
    let policy = policy(headers, Duration::MAX);
    if status == 304 && matches!(response.exit_code, 0 | 1) {
        let response = revalidated(state, request, plan, &mut entry, etag, &policy, epoch).await?;
        retain(state, &key, &slot, &mut entry, policy.store, epoch).await?;
        *retained = entry;
        return Ok(response);
    }
    if response.exit_code != 0 {
        state.rest_counters.errors.fetch_add(1, Ordering::Relaxed);
    }
    let body = body.to_vec();
    let rendered = render(
        state,
        request,
        plan,
        body.clone(),
        response.stderr.clone(),
        response.exit_code,
        "fetched",
    )
    .await?;
    state.journal.confirm(epoch)?;
    if status == 200 && response.exit_code == 0 && policy.store && !rest.rate_limit {
        entry = Some(Entry {
            body,
            stderr: response.stderr,
            etag,
            validated: Some(Instant::now()),
            ttl: policy.ttl,
        });
    } else {
        entry = None;
    }
    retain(state, &key, &slot, &mut entry, true, epoch).await?;
    *retained = entry;
    Ok(rendered)
}

async fn revalidated(
    state: &State,
    request: &GhReadRequest,
    plan: &ReadPlan,
    entry: &mut Option<Entry>,
    etag: Option<String>,
    policy: &Policy,
    epoch: u64,
) -> io::Result<GhReadResponse> {
    let cached = entry
        .as_mut()
        .filter(|entry| entry.etag.is_some())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "REST304 has no retained validated body",
            )
        })?;
    state
        .rest_counters
        .revalidated
        .fetch_add(1, Ordering::Relaxed);
    if etag.is_some() {
        cached.etag = etag;
    }
    cached.validated = Some(Instant::now());
    if policy.specified {
        cached.ttl = policy.ttl;
    }
    let response = render(
        state,
        request,
        plan,
        cached.body.clone(),
        cached.stderr.clone(),
        0,
        "revalidated",
    )
    .await?;
    state.journal.confirm(epoch)?;
    Ok(response)
}

fn required_rest(plan: &ReadPlan) -> io::Result<&ghd_core::rest::RestPlan> {
    plan.rest
        .as_ref()
        .ok_or_else(|| io::Error::other("missing REST plan"))
}

async fn retain(
    state: &State,
    key: &str,
    slot: &Arc<kernal_api::async_engine::Mutex<Option<Entry>>>,
    entry: &mut Option<Entry>,
    store: bool,
    epoch: u64,
) -> io::Result<()> {
    let weight = entry.as_ref().map_or(0, |cached| {
        cached
            .body
            .len()
            .saturating_add(cached.stderr.len())
            .saturating_add(cached.etag.as_ref().map_or(0, String::len))
    });
    let retained = crate::retention::reserve(
        state,
        key,
        crate::retention::CachedSlot::Rest(Arc::clone(slot)),
        if store { weight } else { 0 },
    )
    .await;
    if !store || !retained {
        *entry = None;
    }
    if let Some(entry) = entry {
        crate::persistence::save_rest(state, key, epoch, entry).await?;
    } else {
        crate::persistence::forget(state, key, epoch).await?;
    }
    state.journal.confirm(epoch)
}

async fn fresh(
    state: &State,
    request: &GhReadRequest,
    plan: &ReadPlan,
    entry: Option<&Entry>,
    epoch: u64,
) -> io::Result<Option<GhReadResponse>> {
    let allowed = !plan.rest.as_ref().is_some_and(|rest| rest.force_refresh);
    let Some(cached) = entry.filter(|entry| {
        allowed
            && entry.validated.is_some_and(|at| {
                at.elapsed()
                    < entry
                        .ttl
                        .min(state.policy.command("api", "rest", state.fresh_for))
            })
    }) else {
        return Ok(None);
    };
    state.journal.confirm(epoch)?;
    let response = render(
        state,
        request,
        plan,
        cached.body.clone(),
        cached.stderr.clone(),
        0,
        "fresh",
    )
    .await?;
    state.journal.confirm(epoch)?;
    state.rest_counters.fresh.fetch_add(1, Ordering::Relaxed);
    Ok(Some(response))
}

async fn fetch(
    state: &State,
    request: &GhReadRequest,
    rest: &ghd_core::rest::RestPlan,
    entry: Option<&Entry>,
) -> io::Result<GhReadResponse> {
    let mut args = rest.fetch_args.clone();
    if !rest.rate_limit
        && let Some(etag) = entry.and_then(|entry| entry.etag.as_ref())
    {
        args.extend(["--header".into(), format!("If-None-Match: {etag}")]);
    }
    let permit = state.gh_permits.acquire().await;
    state
        .rest_counters
        .operations
        .fetch_add(1, Ordering::Relaxed);
    if rest.rate_limit {
        state
            .rest_counters
            .introspections
            .fetch_add(1, Ordering::Relaxed);
    }
    let result = gh_executor::execute(
        gh_executor::spec(request, &state.host, &state.github_token, &args),
        permit,
    )
    .await;
    if result.is_err() {
        state.rest_counters.errors.fetch_add(1, Ordering::Relaxed);
    }
    result
}

async fn render(
    state: &State,
    request: &GhReadRequest,
    plan: &ReadPlan,
    body: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
    source: &str,
) -> io::Result<GhReadResponse> {
    let mut response = GhReadResponse {
        stdout: body,
        stderr,
        exit_code: code,
        source: source.into(),
        ..Default::default()
    };
    if code == 0 && (plan.jq.is_some() || plan.template.is_some()) {
        let rendered = gh_render::render(state, request, plan, response.stdout).await?;
        response.stdout = rendered.stdout;
        response.stderr.extend(rendered.stderr);
        response.exit_code = rendered.exit_code;
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn server_policy_forbids_storage_or_freshness_without_losing_body_custody() {
        let fresh = Duration::from_secs(30);
        assert!(!policy("Cache-Control: no-store\n", fresh).store);
        assert!(!policy("Vary: Accept, *\n", fresh).store);
        assert_eq!(
            policy("Cache-Control: no-cache\n", fresh).ttl,
            Duration::ZERO
        );
        assert_eq!(
            policy("Cache-Control: max-age=10\nAge: 7\n", fresh).ttl,
            Duration::from_secs(3)
        );
        assert_eq!(
            policy("Cache-Control: max-age=oops\n", fresh).ttl,
            Duration::ZERO
        );
    }
    #[test]
    fn http_counts_do_not_invent_points_from_responses() {
        let counters = Counters::default();
        counters.operations.fetch_add(2, Ordering::Relaxed);
        counters.revalidated.fetch_add(1, Ordering::Relaxed);
        counters
            .observe("X-RateLimit-Resource: core\nX-RateLimit-Used: 1\nX-RateLimit-Remaining: 99\n")
            .expect("observe");
        let mut doc = BTreeMap::new();
        counters.add_stats(&mut doc).expect("stats");
        assert_eq!(doc["rest_http_operations"], Value::Unsigned(2));
        assert_eq!(doc["rest_304"], Value::Unsigned(1));
        let Value::Object(budgets) = &doc["rest_provider_budgets"] else {
            panic!("budgets")
        };
        let Value::Object(core) = &budgets["core"] else {
            panic!("core")
        };
        assert_eq!(core["used"], Value::Unsigned(1));
    }
}
