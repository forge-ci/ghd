//! Shared headless read cache over canonical JSON resources or exact gh bytes.

use super::server::{State, read_epoch};
use crate::{gh_executor, gh_render, run_collection};
use ghd_core::{
    proto::{GhReadRequest, GhReadResponse},
    resource::{self, ReadPlan},
};
use kernal_api::http_server::Response;
use prost::Message;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Entry {
    pub response: GhReadResponse,
    pub fields: Vec<String>,
    pub validated: Option<Instant>,
    pub etag: Option<String>,
    pub ttl: Duration,
}

fn reply(status: u16, response: GhReadResponse) -> Response {
    Response::new(status, response.encode_to_vec())
        .and_then(|response| response.with_header("content-type", "application/x-protobuf"))
        .unwrap_or_default()
}

fn failed(status: u16, error: impl ToString) -> Response {
    reply(
        status,
        GhReadResponse {
            error: error.to_string(),
            ..Default::default()
        },
    )
}

pub(crate) fn key(request: &GhReadRequest, plan: &ReadPlan) -> String {
    context_key(
        request,
        &plan.key_args,
        plan.worktree_dependent,
        plan.resource,
        plan.repo.as_deref().unwrap_or_default(),
    )
}

pub(crate) fn context_key(
    request: &GhReadRequest,
    args: &[String],
    worktree_dependent: bool,
    normalize_repo: bool,
    repo: &str,
) -> String {
    let mut canonical = request.clone();
    canonical.args = args.to_vec();
    if !worktree_dependent {
        canonical.cwd.clear();
        canonical.worktree_stamp.clear();
    }
    if normalize_repo {
        canonical.default_repo = repo.to_owned();
        canonical.environment.retain(|v| v.name != "GH_REPO");
    }
    canonical.environment.sort_by(|a, b| a.name.cmp(&b.name));
    kernal_api::hash::blake3_bytes(&canonical.encode_to_vec()).to_hex()
}

pub async fn handle(state: &State, body: &[u8]) -> Response {
    let request = match GhReadRequest::decode(body) {
        Ok(request) => request,
        Err(error) => return failed(400, error),
    };
    match run(state, &request).await {
        Ok(response) => {
            state.control.answer(&response.source);
            reply(200, response)
        }
        Err(error) => failed(409, error),
    }
}

fn validated_plan(state: &State, request: &GhReadRequest) -> io::Result<ReadPlan> {
    gh_executor::validate(request)?;
    let plan = resource::prepare(
        &request.args,
        &state.host,
        (!request.default_repo.is_empty()).then_some(request.default_repo.as_str()),
    )?
    .ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "gh read shape is not cacheable",
        )
    })?;
    if plan.host != state.host || request.hostname != state.host {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "gh read host does not match daemon identity",
        ));
    }
    Ok(plan)
}

async fn run(state: &State, request: &GhReadRequest) -> io::Result<GhReadResponse> {
    let plan = validated_plan(state, request)?;
    if plan.rest.is_some() {
        return crate::rest_reads::run(state, request, &plan).await;
    }
    let epoch = read_epoch(state).await?;
    let _admission = crate::retention::admit(state).await?;
    let key = crate::retention::key("gh", key(request, &plan).as_bytes());
    let slot = crate::retention::gh(state, &key).await;
    let mut retained = slot.lock().await;
    let mut entry = retained.take();
    if entry.is_none() {
        entry = crate::persistence::restore_gh(state, &key, epoch).await?;
    }
    if let Some(cached) = entry.as_ref()
        && cached.validated.is_some_and(|at| {
            at.elapsed()
                < state
                    .policy
                    .resource(&plan, &cached.response.stdout, state.fresh_for)
                    .min(cached.ttl)
        })
        && (!plan.resource || plan.project(&cached.response.stdout).is_ok())
    {
        state.journal.confirm(epoch)?;
        let mut response = project(state, request, &plan, &cached.response).await?;
        state.journal.confirm(epoch)?;
        response.source = "fresh".into();
        *retained = entry;
        return Ok(response);
    }
    let extra = entry
        .as_ref()
        .map_or(&[][..], |cached| cached.fields.as_slice());
    let fetch_args = if plan.resource {
        plan.with_fields(extra)
    } else {
        plan.fetch_args.clone()
    };
    let Fetched {
        response,
        etag,
        ttl,
        store,
    } = fetch(state, request, &plan, &fetch_args, entry.as_ref()).await?;
    state.journal.confirm(epoch)?;
    let checks_result = plan.kind == "pr"
        && plan.action == "checks"
        && (response.exit_code == 8
            || response.exit_code == 1 && plan.resource && plan.project(&response.stdout).is_ok());
    let cacheable_status = response.exit_code == 0 || checks_result;
    if !cacheable_status {
        return Ok(response);
    }
    let projected = project(state, request, &plan, &response).await?;
    state.journal.confirm(epoch)?;
    let fields = if plan.resource {
        plan.fetch_fields
            .iter()
            .chain(extra)
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    entry = retain(
        state,
        &key,
        &slot,
        Entry {
            response,
            fields,
            validated: Some(Instant::now()),
            etag,
            ttl,
        },
        store,
        epoch,
    )
    .await?;
    *retained = entry;
    Ok(projected)
}

async fn retain(
    state: &State,
    key: &str,
    slot: &Arc<kernal_api::async_engine::Mutex<Option<Entry>>>,
    entry: Entry,
    store: bool,
    epoch: u64,
) -> io::Result<Option<Entry>> {
    let weight = entry
        .response
        .encoded_len()
        .saturating_add(
            entry
                .fields
                .iter()
                .map(|field| field.len().saturating_add(32))
                .sum::<usize>(),
        )
        .saturating_add(entry.etag.as_ref().map_or(0, String::len));
    let retain = crate::retention::reserve(
        state,
        key,
        crate::retention::CachedSlot::Gh(Arc::clone(slot)),
        if store { weight } else { 0 },
    )
    .await
        && store;
    let entry = retain.then_some(entry);
    if let Some(entry) = entry.as_ref() {
        crate::persistence::save_gh(state, key, epoch, entry).await?;
    } else {
        crate::persistence::forget(state, key, epoch).await?;
    }
    state.journal.confirm(epoch)?;
    Ok(entry)
}

struct Fetched {
    response: GhReadResponse,
    etag: Option<String>,
    ttl: Duration,
    store: bool,
}

async fn fetch(
    state: &State,
    request: &GhReadRequest,
    plan: &ReadPlan,
    args: &[String],
    entry: Option<&Entry>,
) -> io::Result<Fetched> {
    let retained = entry.map_or(&[][..], |cached| cached.fields.as_slice());
    if let Some(path) = run_collection::path(plan, retained) {
        let etag = entry.and_then(|cached| cached.etag.as_deref());
        let mut api_args = vec![
            "api".into(),
            path.trim_start_matches('/').into(),
            "--hostname".into(),
            state.host.clone(),
            "--method".into(),
            "GET".into(),
            "--include".into(),
        ];
        if let Some(etag) = etag {
            api_args.extend(["--header".into(), format!("If-None-Match: {etag}")]);
        }
        let response = execute(state, request, &api_args).await?;
        if let Ok((status, next_etag, body)) = run_collection::included_response(&response.stdout) {
            let headers =
                std::str::from_utf8(&response.stdout[..response.stdout.len() - body.len()])
                    .map_err(io::Error::other)?;
            let policy = crate::rest_reads::policy(headers, Duration::MAX);
            if status == 304 {
                let (response, etag) = revalidated(entry, next_etag)?;
                let ttl = if policy.specified {
                    policy.ttl
                } else {
                    entry.map_or(policy.ttl, |entry| entry.ttl)
                };
                return Ok(Fetched {
                    response,
                    etag,
                    ttl,
                    store: policy.store,
                });
            }
            if status == 200 && response.exit_code == 0 {
                return Ok(Fetched {
                    response: GhReadResponse {
                        stdout: run_collection::translate(body)?,
                        source: "fetched".into(),
                        ..Default::default()
                    },
                    etag: next_etag,
                    ttl: policy.ttl,
                    store: policy.store,
                });
            }
        } else if response.exit_code == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "gh api returned an invalid included response",
            ));
        }
        // Native gh owns exact command error rendering. This is a safe read;
        // retrying an HTTP error as that command cannot repeat a mutation.
    }
    Ok(Fetched {
        response: execute(state, request, args).await?,
        etag: None,
        ttl: Duration::MAX,
        store: true,
    })
}

async fn execute(
    state: &State,
    request: &GhReadRequest,
    args: &[String],
) -> io::Result<GhReadResponse> {
    let category = if args.first().is_some_and(|arg| arg == "api") {
        ghd_core::governor::rest_category(args.get(1).map_or("", String::as_str))
    } else {
        "unknown"
    };
    let dispatch = state
        .control
        .enter(category, false, false, crate::governance::Event::Process)
        .await?;
    gh_executor::governed(
        gh_executor::spec(request, &state.host, &state.github_token, args),
        state.gh_permits.acquire().await,
        dispatch,
        args.first().is_some_and(|arg| arg == "api") && args.iter().any(|arg| arg == "--include"),
    )
    .await
}

fn revalidated(
    entry: Option<&Entry>,
    next_etag: Option<String>,
) -> io::Result<(GhReadResponse, Option<String>)> {
    let cached = entry.filter(|entry| entry.etag.is_some()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "collection 304 has no retained body",
        )
    })?;
    let mut response = cached.response.clone();
    response.source = "revalidated".into();
    Ok((response, next_etag.or_else(|| cached.etag.clone())))
}

async fn project(
    state: &State,
    request: &GhReadRequest,
    plan: &ReadPlan,
    response: &GhReadResponse,
) -> io::Result<GhReadResponse> {
    let mut response = response.clone();
    if plan.resource {
        response.stdout = plan.project(&response.stdout)?;
        if let Some(limit) = plan.limit {
            let value = kernal_api::json::parse(&response.stdout).map_err(io::Error::other)?;
            response.limit_reached =
                matches!(value, kernal_api::json::Value::Array(rows) if rows.len() >= limit);
        }
        if plan.jq.is_some() || plan.template.is_some() {
            let rendered = gh_render::render(state, request, plan, response.stdout).await?;
            response.stdout = rendered.stdout;
            response.stderr.extend(rendered.stderr);
            // A renderer failure cannot turn pending-checks exit 8 into success.
            if rendered.exit_code != 0 {
                response.exit_code = rendered.exit_code;
            }
        }
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collection_304_keeps_matching_body_and_rejects_missing_custody() {
        let entry = Entry {
            response: GhReadResponse {
                stdout: b"retained rows".to_vec(),
                ..Default::default()
            },
            fields: Vec::new(),
            validated: Some(Instant::now()),
            etag: Some("\"v1\"".into()),
            ttl: Duration::MAX,
        };
        let (response, etag) = revalidated(Some(&entry), None).expect("retained");
        assert_eq!(response.stdout, b"retained rows");
        assert_eq!(response.source, "revalidated");
        assert_eq!(etag, entry.etag);
        assert!(revalidated(None, Some("\"v1\"".into())).is_err());
        let unvalidated = Entry {
            etag: None,
            ..entry
        };
        assert!(revalidated(Some(&unvalidated), None).is_err());
    }
}
