//! Versioned repository snapshots with explicit authority and stale planning.

use crate::{
    gh_executor, gh_reads, governance, retention, run_collection,
    server::{State, read_epoch},
};
use ghd_core::{
    governor::header,
    proto::{GhReadRequest, SnapshotRequest, SnapshotResponse},
    snapshot::{self, Spec},
};
use kernal_api::{http_server::Response, json};
use prost::Message;
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) struct Entry {
    repository: Vec<u8>,
    truncated: bool,
    captured: Instant,
    captured_at_ms: u64,
    upstream_age_ms: Option<u64>,
    stale: bool,
    ttl: Duration,
    store: bool,
    reason: String,
}
impl Entry {
    fn age(&self) -> Option<u64> {
        self.upstream_age_ms.map(|age| {
            age.saturating_add(
                u64::try_from(self.captured.elapsed().as_millis()).unwrap_or(u64::MAX),
            )
        })
    }
    fn response(&self, source: &str, stale: bool, reason: &str) -> io::Result<SnapshotResponse> {
        let repo = json::parse(&self.repository).map_err(io::Error::other)?;
        Ok(SnapshotResponse {
            json: snapshot::envelope(
                repo,
                source,
                self.age(),
                stale,
                self.truncated,
                self.captured_at_ms,
                reason,
            )?,
            source: source.into(),
            age_ms: self.age(),
            stale,
            truncated: self.truncated,
            ..Default::default()
        })
    }
}

fn refused(error: impl ToString, age_ms: Option<u64>) -> SnapshotResponse {
    SnapshotResponse {
        source: "refused".into(),
        exit_code: 3,
        error: error.to_string(),
        age_ms,
        ..Default::default()
    }
}

pub(crate) async fn handle(state: &State, body: &[u8]) -> Response {
    let response = match SnapshotRequest::decode(body) {
        Ok(request) => match run(state, request).await {
            Ok(response) => response,
            Err(error) => refused(error, None),
        },
        Err(error) => refused(error, None),
    };
    crate::ledger::answer(
        &response.source,
        response.json.len(),
        response.exit_code == 0 && response.error.is_empty(),
    );
    state.control.answer(&response.source);
    Response::new(200, response.encode_to_vec())
        .and_then(|response| response.with_header("content-type", "application/x-protobuf"))
        .unwrap_or_default()
}

fn validated_request(state: &State, request: SnapshotRequest) -> io::Result<(GhReadRequest, Spec)> {
    if request.version != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "snapshot protocol unsupported; restart after upgrading",
        ));
    }
    let backend = request.backend.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "snapshot backend is missing")
    })?;
    gh_executor::validate(&backend)?;
    if backend.hostname != state.host {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "snapshot host does not match identity",
        ));
    }
    let spec = Spec {
        hostname: state.host.clone(),
        owner: request.owner.to_ascii_lowercase(),
        repo: request.repo.to_ascii_lowercase(),
        limit: request.limit,
        fresh: request.fresh,
        allow_stale: request.allow_stale,
    };
    spec.validate()?;
    Ok((backend, spec))
}

async fn run(state: &State, request: SnapshotRequest) -> io::Result<SnapshotResponse> {
    let (backend, spec) = validated_request(state, request)?;
    let arrived = Instant::now();
    let epoch = read_epoch(state).await?;
    let _admission = retention::admit(state).await?;
    let args = spec.args();
    let repo = format!("{}/{}", spec.owner, spec.repo);
    let key = retention::key(
        "snapshot",
        gh_reads::context_key(&backend, &args, false, true, &repo).as_bytes(),
    );
    let slot = retention::snapshot(state, &key).await;
    if slot.try_lock().is_err() {
        crate::ledger::waited();
    }
    let mut retained = slot.lock().await;
    let mut entry = retained.take();
    let ttl = crate::policy::ttl(state.policy.command("snapshot", "read", state.fresh_for));
    if !spec.fresh
        && !crate::policy::current().bypass
        && let Some(cached) = entry.as_ref()
        && !cached.stale
        && (cached.captured >= arrived
            || cached
                .age()
                .is_some_and(|age| u128::from(age) < ttl.min(cached.ttl).as_millis()))
    {
        let response = cached.response(
            if cached.captured >= arrived {
                "shared"
            } else {
                "fresh"
            },
            false,
            "",
        )?;
        state.journal.confirm(epoch)?;
        *retained = entry;
        return Ok(response);
    }
    let outcome = fetch(state, &backend, &spec, &args).await;
    state.journal.confirm(epoch)?;
    let response = match outcome {
        Ok(next) => {
            let response = if next.stale && !spec.allow_stale {
                refused(
                    "origin returned a stale snapshot; --allow-stale is required for planning",
                    next.age(),
                )
            } else {
                next.response(
                    if next.stale { "stale" } else { "fetched" },
                    next.stale,
                    &next.reason,
                )?
            };
            entry = Some(next);
            response
        }
        Err(error) => match entry.as_ref() {
            Some(cached) if spec.allow_stale => {
                cached.response("stale", true, &bounded_reason(&error.to_string()))?
            }
            cached => refused(error, cached.and_then(Entry::age)),
        },
    };
    state.journal.confirm(epoch)?;
    if entry.as_ref().is_some_and(|entry| !entry.store) {
        entry = None;
    }
    let weight = entry.as_ref().map_or(0, |entry| {
        entry.repository.len().saturating_add(entry.reason.len())
    });
    if retention::reserve(
        state,
        &key,
        retention::CachedSlot::Snapshot(Arc::clone(&slot)),
        weight,
    )
    .await
    {
        *retained = entry;
    }
    state.journal.confirm(epoch)?;
    Ok(response)
}

fn bounded_reason(reason: &str) -> String {
    reason.chars().take(512).collect()
}

async fn fetch(
    state: &State,
    backend: &GhReadRequest,
    spec: &Spec,
    args: &[String],
) -> io::Result<Entry> {
    let dispatch = state
        .control
        .enter("graphql", spec.fresh, false, governance::Event::Process)
        .await?;
    let response = gh_executor::governed(
        gh_executor::spec(backend, &state.host, &state.github_token, args),
        state.gh_permits.acquire().await,
        dispatch,
        true,
    )
    .await?;
    let (status, _, body) = run_collection::included_response(&response.stdout)?;
    if status != 200 || response.exit_code != 0 {
        return Err(io::Error::other(format!(
            "snapshot origin failed (HTTP {status}, exit {}): {}",
            response.exit_code,
            bounded_reason(&String::from_utf8_lossy(&response.stderr))
        )));
    }
    let headers = std::str::from_utf8(&response.stdout[..response.stdout.len() - body.len()])
        .map_err(io::Error::other)?;
    let (stale, age) = origin_age(headers)?;
    let policy = crate::rest_reads::policy(headers, Duration::MAX);
    let (repo, truncated) = snapshot::repository(body, &format!("{}/{}", spec.owner, spec.repo))?;
    let repository = json::encode(&repo, json::Layout::Compact).map_err(io::Error::other)?;
    let captured_at_ms = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_millis(),
    )
    .map_err(io::Error::other)?;
    Ok(Entry {
        repository,
        truncated,
        captured: Instant::now(),
        captured_at_ms,
        upstream_age_ms: age,
        stale,
        ttl: policy.ttl,
        store: policy.store,
        reason: if stale {
            "origin cache marked stale or returned positive Age".into()
        } else {
            String::new()
        },
    })
}

fn origin_age(headers: &str) -> io::Result<(bool, Option<u64>)> {
    let age = header(headers, "age")
        .map(|age| age.parse::<u64>().map(|age| age.saturating_mul(1000)))
        .transpose()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid snapshot Age header"))?;
    let marked = ["x-ghgate-stale", "x-ghd-stale"].iter().any(|name| {
        header(headers, name)
            .is_some_and(|value| !value.is_empty() && value != "0" && value != "false")
    }) || header(headers, "warning").is_some_and(|value| {
        value
            .split(',')
            .any(|part| part.trim_start().starts_with("110 "))
    });
    let stale = marked || age.is_some_and(|age| age > 0);
    Ok((
        stale,
        if marked && age.is_none() {
            None
        } else {
            Some(age.unwrap_or(0))
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downstream_age_and_stale_markers_are_not_freshness_authority() {
        assert_eq!(origin_age("").expect("origin"), (false, Some(0)));
        assert_eq!(origin_age("Age: 3").expect("aged"), (true, Some(3000)));
        assert_eq!(
            origin_age("X-Ghgate-Stale: true").expect("unknown age"),
            (true, None)
        );
        assert_eq!(
            origin_age("Warning: 110 proxy stale").expect("warning"),
            (true, None)
        );
        assert!(origin_age("Age: invalid").is_err());
    }
}
