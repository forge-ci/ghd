//! Bounded payload-free request evidence; raw arguments are never retained.
use crate::{governance, server::State};
use ghd_core::proto::GhReadRequest;
use kernal_api::{
    http_server::{Request, Response},
    json::{self, Value},
};
use prost::Message;
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Mutex},
    time::Instant,
};
const CAPACITY: usize = 256;
#[derive(Default)]
struct Outcome {
    source: String,
    bytes: usize,
    waited: bool,
    unknown: bool,
    dispatches: Option<u64>,
}
kernal_api::task_local! { static CURRENT: Arc<Mutex<Outcome>> = LEDGER_STORAGE; }
pub(crate) struct Ledger {
    started: Instant,
    capture: bool,
    rows: Mutex<(VecDeque<Value>, u64)>,
}
impl Ledger {
    pub(crate) fn new() -> io::Result<Self> {
        let capture = match ghd_core::config::value("GHD_LOG_ARGS")?.as_deref() {
            None | Some("0") => false,
            Some("1") => true,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "GHD_LOG_ARGS must be 0 or 1",
                ));
            }
        };
        Ok(Self {
            started: Instant::now(),
            capture,
            rows: Mutex::default(),
        })
    }
    pub(crate) fn summary(&self, evictions: u64) -> io::Result<Value> {
        let Value::Object(mut doc) = self.document(evictions)? else {
            return Err(io::Error::other("ledger schema"));
        };
        doc.remove("entries");
        Ok(Value::Object(doc))
    }
    fn begin<'a>(&'a self, request: &Request, path: &str) -> Event<'a> {
        let backend = if path == "/v1/read/gh" {
            GhReadRequest::decode(request.body()).ok()
        } else {
            None
        };
        let args = backend.as_ref().map(|r| &r.args);
        let kind = operation_kind(path, request.body());
        let eligible = kind == "read"
            && !crate::policy::current().active()
            && match path {
                "/v2/snapshot" => ghd_core::proto::SnapshotRequest::decode(request.body())
                    .is_ok_and(|r| !r.fresh && !r.allow_stale),
                "/v1/read/gh" => backend.as_ref().is_some_and(|r| {
                    ghd_core::resource::prepare(&r.args, &r.hostname, Some(&r.default_repo))
                        .ok()
                        .flatten()
                        .is_some_and(|p| {
                            p.rest
                                .is_none_or(|rest| !rest.force_refresh && !rest.rate_limit)
                        })
                }),
                _ => true,
            };
        let command = match path {
            "/v1/pr/view" => "pr view".into(),
            "/v1/graphql" => "api graphql".into(),
            "/v2/snapshot" => "snapshot read".into(),
            _ => args
                .as_ref()
                .map_or_else(|| "unknown".into(), |args| label(args)),
        };
        Event {
            ledger: self,
            started: Instant::now(),
            outcome: Arc::default(),
            command,
            kind,
            eligible,
            key: kernal_api::hash::blake3_bytes(request.body()).to_hex(),
            caller: governance::caller(),
            args: if self.capture {
                args.map(|args| ghd_core::diagnostics::redact(args))
            } else {
                None
            },
        }
    }
    pub(crate) fn document(&self, evictions: u64) -> io::Result<Value> {
        let rows = self
            .rows
            .lock()
            .map_err(|_| io::Error::other("ledger poisoned"))?;
        let mut commands: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
        for row in &rows.0 {
            let Value::Object(row) = row else {
                continue;
            };
            let (Some(Value::String(command)), Some(Value::String(source))) =
                (row.get("command"), row.get("source"))
            else {
                continue;
            };
            let counts = commands.entry(command.clone()).or_default();
            for key in ["requests", source.as_str()] {
                let count = counts.entry(key.into()).or_insert(Value::Unsigned(0));
                if let Value::Unsigned(count) = count {
                    *count = count.saturating_add(1);
                }
            }
        }
        Ok(Value::Object(BTreeMap::from([
            ("commands".into(), Value::Object(commands.into_iter().map(|(key, counts)| (key, Value::Object(counts))).collect())),
            ("schema".into(), Value::Unsigned(1)), ("capacity".into(), Value::Unsigned(CAPACITY as u64)),
            ("dropped".into(), Value::Unsigned(rows.1)), ("evictions".into(), Value::Unsigned(evictions)),
            ("entries".into(), Value::Array(rows.0.iter().cloned().collect())),
            ("scope".into(), Value::String("Bounded daemon-managed request window; dispatches are logical managed operations, not exact provider requests/points; null means unknown.".into())),
        ])))
    }
}
fn operation_kind(path: &str, body: &[u8]) -> &'static str {
    if path != "/v1/graphql" {
        return "read";
    }
    let Ok(request) = ghd_core::proto::GraphqlRequest::decode(body) else {
        return "unknown";
    };
    match ghd_core::operation::classify(
        &request.query,
        (!request.operation_name.is_empty()).then_some(request.operation_name.as_str()),
    ) {
        Ok(ghd_core::operation::Operation::Read) => "read",
        Ok(ghd_core::operation::Operation::Write) => "write",
        Err(_) => "unknown",
    }
}
fn label(args: &[String]) -> String {
    match args
        .iter()
        .take(2)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [
            kind @ ("issue" | "pr" | "run"),
            action @ ("view" | "list" | "checks"),
        ] => format!("{kind} {action}"),
        ["api", "graphql"] => "api graphql".into(),
        ["api", _] => "api rest".into(),
        _ => "other read".into(),
    }
}
struct Event<'a> {
    ledger: &'a Ledger,
    started: Instant,
    outcome: Arc<Mutex<Outcome>>,
    command: String,
    kind: &'static str,
    eligible: bool,
    key: String,
    caller: String,
    args: Option<Vec<String>>,
}
impl Drop for Event<'_> {
    fn drop(&mut self) {
        let (Ok(outcome), Ok(mut rows)) = (self.outcome.lock(), self.ledger.rows.lock()) else {
            return;
        };
        let source = match outcome.source.as_str() {
            "fetched" if self.kind == "write" => "write",
            "fresh" | "shared" if outcome.waited => "coalesced",
            "fresh" | "shared" => "hit",
            "revalidated" => "304",
            "fetched" => "miss",
            "stale" => "stale",
            _ => "refused",
        };
        let dispatches = if outcome.unknown {
            None
        } else {
            outcome
                .dispatches
                .or_else(|| matches!(source, "hit" | "coalesced").then_some(0))
        };
        let n = |value: u128| Value::Unsigned(u64::try_from(value).unwrap_or(u64::MAX));
        let row = Value::Object(BTreeMap::from([
            ("at_ms".into(), n(self.ledger.started.elapsed().as_millis())),
            ("latency_us".into(), n(self.started.elapsed().as_micros())),
            ("response_bytes".into(), n(outcome.bytes as u128)),
            ("source".into(), Value::String(source.into())),
            ("operation_kind".into(), Value::String(self.kind.into())),
            ("cache_eligible".into(), Value::Bool(self.eligible)),
            ("command".into(), Value::String(self.command.clone())),
            ("key".into(), Value::String(self.key.clone())),
            ("caller".into(), Value::String(self.caller.clone())),
            (
                "args".into(),
                self.args.as_ref().map_or(Value::Null, |args| {
                    Value::Array(args.iter().cloned().map(Value::String).collect())
                }),
            ),
            (
                "upstream_dispatches".into(),
                dispatches.map_or(Value::Null, Value::Unsigned),
            ),
        ]));
        if rows.0.len() == CAPACITY {
            rows.0.pop_front();
            rows.1 = rows.1.saturating_add(1);
        }
        rows.0.push_back(row);
    }
}
pub(crate) fn answer(source: &str, bytes: usize, success: bool) {
    let _ = CURRENT.try_with(|outcome| {
        if let Ok(mut outcome) = outcome.lock() {
            outcome.source = if success { source } else { "refused" }.into();
            outcome.bytes = bytes;
        }
    });
}
pub(crate) fn waited() {
    let _ = CURRENT.try_with(|outcome| {
        if let Ok(mut outcome) = outcome.lock() {
            outcome.waited = true;
        }
    });
}
pub(crate) struct Capture(Arc<Mutex<Outcome>>);
pub(crate) fn capture() -> Option<Capture> {
    CURRENT.try_with(|v| Capture(Arc::clone(v))).ok()
}
impl Capture {
    pub(crate) fn dispatched(&self) {
        if let Ok(mut value) = self.0.lock() {
            value.dispatches = Some(value.dispatches.unwrap_or(0).saturating_add(1));
        }
    }
}
pub(crate) fn unknown_dispatches() {
    let _ = CURRENT.try_with(|v| {
        if let Ok(mut v) = v.lock() {
            v.unknown = true;
        }
    });
}
pub(crate) async fn track(state: &State, request: &Request, path: &str) -> Response {
    let event = state.ledger.begin(request, path);
    CURRENT
        .scope(Arc::clone(&event.outcome), async {
            match path {
                "/v1/pr/view" => crate::server::pr_view(state, request.body()).await,
                "/v1/graphql" => crate::server::graphql(state, request.body()).await,
                "/v1/read/gh" => crate::gh_reads::handle(state, request.body()).await,
                "/v2/snapshot" => crate::snapshots::handle(state, request.body()).await,
                _ => Response::default(),
            }
        })
        .await
}
pub(crate) async fn report(state: &State, html: bool) -> Response {
    let evictions = state.retained.lock().await.evictions();
    let body = state
        .ledger
        .document(evictions)
        .and_then(|value| json::encode(&value, json::Layout::Pretty).map_err(io::Error::other));
    match body {
        Ok(body) => {
            let body = if html {
                format!("<!doctype html><html><head><meta charset=utf-8><title>ghd diagnostics</title></head><body><h1>ghd request evidence</h1><p>Private read-only snapshot. Counts are managed logical dispatches; provider points and unknown work are not inferred.</p><pre>{}</pre></body></html>", escape(&String::from_utf8_lossy(&body))).into_bytes()
            } else {
                body
            };
            Response::new(200, body)
                .and_then(|r| {
                    r.with_header(
                        "content-type",
                        if html {
                            "text/html; charset=utf-8"
                        } else {
                            "application/json"
                        },
                    )
                })
                .and_then(|r| r.with_header("cache-control", "no-store"))
                .and_then(|r| {
                    r.with_header(
                        "content-security-policy",
                        "default-src 'none'; frame-ancestors 'none'",
                    )
                })
                .unwrap_or_default()
        }
        Err(_) => Response::default(),
    }
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutation_and_selected_mixed_operation_are_never_cache_eligible() {
        let mut request = ghd_core::proto::GraphqlRequest { query: "query Read { viewer { login } } mutation Write { clearLabelsFromLabelable(input:{labelableId:\"x\"}) { clientMutationId } }".into(), operation_name: "Write".into(), ..Default::default() };
        assert_eq!(
            operation_kind("/v1/graphql", &request.encode_to_vec()),
            "write"
        );
        request.operation_name = "Read".into();
        assert_eq!(
            operation_kind("/v1/graphql", &request.encode_to_vec()),
            "read"
        );
        request.operation_name.clear();
        assert_eq!(
            operation_kind("/v1/graphql", &request.encode_to_vec()),
            "unknown"
        );
    }
}
