//! Upstream GitHub: conditional REST sentinels and the batched GraphQL reader.
//!
//! A 304 to a conditional REST GET costs no rate-limit budget, so every
//! cached GraphQL answer is guarded by its entity's REST ETag.

use kernal_api::async_engine::{self, BroadcastSender, Mutex};
use kernal_api::http::{self, Method, Request};
use kernal_api::json::{self, Layout, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

const USER_AGENT: &str = concat!("ghd/", env!("CARGO_PKG_VERSION"));

pub type Fields = BTreeMap<String, Value>;

#[derive(Default)]
pub struct Counters {
    pub sentinel_304: AtomicU64,
    pub sentinel_200: AtomicU64,
    pub graphql_queries: AtomicU64,
    pub graphql_prs: AtomicU64,
    pub graphql_cost: AtomicU64,
    pub graphql_remaining: AtomicI64,
    /// `api graphql` calls that went upstream.
    pub graphql_passthrough: AtomicU64,
    pub core_remaining: AtomicI64,
}

pub struct Upstream {
    http: http::Client,
    auth: String,
    api: String,
    graphql_url: String,
    pub counters: Counters,
    batches: Mutex<HashMap<(String, String), Batch>>,
    window: Duration,
    batch_permits: async_engine::Semaphore,
}

type BatchResult = Arc<Result<HashMap<u64, Fields>, String>>;

struct Batch {
    prs: BTreeMap<u64, BTreeSet<String>>,
    done: BroadcastSender<BatchResult>,
}

impl Upstream {
    pub fn new(identity: &ghd_core::identity::Identity, window: Duration) -> io::Result<Self> {
        let limits = http::Limits {
            max_body_bytes: 32 * 1024 * 1024,
            // Renamed/transferred repos answer 301; the hop stays on
            // the same API origin, so the Authorization header is kept.
            max_redirects: 3,
            ..http::Limits::default()
        };
        Ok(Self {
            http: http::Client::new(limits)?,
            auth: format!("Bearer {}", identity.token()),
            api: identity.api_base(),
            graphql_url: identity.graphql_url(),
            counters: Counters::default(),
            batches: Mutex::new(HashMap::new()),
            window,
            batch_permits: async_engine::Semaphore::new(32),
        })
    }

    /// Cache tests must fail locally on an unexpected miss, never contact GitHub.
    #[cfg(test)]
    pub(crate) fn disable_network_for_test(&mut self) {
        self.set_endpoint_for_test("http://127.0.0.1:0");
    }

    #[cfg(test)]
    pub(crate) fn set_endpoint_for_test(&mut self, base: &str) {
        self.api = base.into();
        self.graphql_url = format!("{base}/graphql");
    }

    /// Conditional GET. Returns `(status, etag)`; a 304 costs no budget.
    pub(crate) async fn sentinel(
        &self,
        path: &str,
        etag: Option<&str>,
    ) -> io::Result<(u16, Option<String>, crate::rest_reads::Policy)> {
        let url = format!("{}{path}", self.api);
        let mut headers = vec![
            ("Authorization", self.auth.as_str()),
            ("User-Agent", USER_AGENT),
            ("Accept", "application/vnd.github+json"),
        ];
        if let Some(tag) = etag {
            headers.push(("If-None-Match", tag));
        }
        let response = self
            .http
            .execute(Request {
                method: Method::Get,
                url: &url,
                headers: &headers,
                body: &[],
            })
            .await?;
        let status = response.status();
        let new_etag = header_str(&response, "etag");
        if let Some(left) =
            header_str(&response, "x-ratelimit-remaining").and_then(|v| v.parse().ok())
        {
            self.counters.core_remaining.store(left, Ordering::Relaxed);
        }
        let headers = ["cache-control", "age", "vary"]
            .into_iter()
            .filter_map(|name| header_str(&response, name).map(|value| format!("{name}: {value}")))
            .collect::<Vec<_>>()
            .join("\n");
        let policy = crate::rest_reads::policy(&headers, Duration::MAX);
        response.into_bytes().await?;
        match status {
            304 => self.counters.sentinel_304.fetch_add(1, Ordering::Relaxed),
            _ => self.counters.sentinel_200.fetch_add(1, Ordering::Relaxed),
        };
        Ok((status, new_etag, policy))
    }

    /// Fetch one PR's fields, sharing a single GraphQL query with every other
    /// PR of the same repo requested inside the batch window.
    pub async fn pull_request(
        self: &Arc<Self>,
        owner: &str,
        repo: &str,
        number: u64,
        fields: &BTreeSet<String>,
    ) -> Result<Fields, String> {
        let capacity = async_engine::timeout(Duration::from_secs(1), self.batch_permits.acquire())
            .await
            .map_err(|_| "GraphQL batch capacity is busy; retry".to_string())?;
        let key = (owner.to_string(), repo.to_string());
        let mut receiver = {
            let mut batches = self.batches.lock().await;
            if let Some(batch) = batches.get_mut(&key) {
                if batch.prs.len() >= 32 && !batch.prs.contains_key(&number) {
                    return Err("GraphQL batch member capacity is busy; retry".into());
                }
                drop(capacity);
                batch
                    .prs
                    .entry(number)
                    .or_default()
                    .extend(fields.iter().cloned());
                batch.done.subscribe()
            } else {
                let (done, receiver) =
                    async_engine::broadcast_channel(1).map_err(|e| e.to_string())?;
                let mut prs = BTreeMap::new();
                prs.insert(number, fields.clone());
                batches.insert(key.clone(), Batch { prs, done });
                let this = Arc::clone(self);
                async_engine::launch(async move {
                    // Cancellation of every subscriber cannot free a live batch's capacity.
                    let _capacity = capacity;
                    this.flush(key).await;
                })
                .detach();
                receiver
            }
        };
        let result = receiver
            .recv()
            .await
            .map_err(|e| format!("batch dropped: {e:?}"))?;
        match result.as_ref() {
            Ok(prs) => prs
                .get(&number)
                .cloned()
                .ok_or_else(|| format!("pull request #{number} not found")),
            Err(e) => Err(e.clone()),
        }
    }

    async fn flush(self: Arc<Self>, key: (String, String)) {
        async_engine::sleep(self.window).await;
        let Some(batch) = self.batches.lock().await.remove(&key) else {
            return;
        };
        let query = build_query(&key.0, &key.1, &batch.prs);
        let result = self
            .graphql(&query)
            .await
            .and_then(|data| split(&data, &batch.prs));
        self.counters
            .graphql_queries
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .graphql_prs
            .fetch_add(batch.prs.len() as u64, Ordering::Relaxed);
        let _ = batch.done.send(Arc::new(result));
    }

    /// Send a caller's `/graphql` body as-is; `(status, response body)`.
    pub async fn graphql_raw(&self, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
        let url = self.graphql_url.clone();
        let headers = [
            ("Authorization", self.auth.as_str()),
            ("User-Agent", USER_AGENT),
            ("Content-Type", "application/json"),
        ];
        let response = self
            .http
            .execute(Request {
                method: Method::Post,
                url: &url,
                headers: &headers,
                body,
            })
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status();
        if let Some(left) =
            header_str(&response, "x-ratelimit-remaining").and_then(|v| v.parse().ok())
        {
            self.counters
                .graphql_remaining
                .store(left, Ordering::Relaxed);
        }
        let bytes = response.into_bytes().await.map_err(|e| e.to_string())?;
        self.counters
            .graphql_passthrough
            .fetch_add(1, Ordering::Relaxed);
        Ok((status, bytes.to_vec()))
    }

    async fn graphql(&self, query: &str) -> Result<Value, String> {
        let mut doc = BTreeMap::new();
        doc.insert("query".to_string(), Value::String(query.to_string()));
        let body = json::encode(&Value::Object(doc), Layout::Compact).map_err(|e| e.to_string())?;
        let url = self.graphql_url.clone();
        let headers = [
            ("Authorization", self.auth.as_str()),
            ("User-Agent", USER_AGENT),
            ("Content-Type", "application/json"),
        ];
        let response = self
            .http
            .execute(Request {
                method: Method::Post,
                url: &url,
                headers: &headers,
                body: &body,
            })
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status();
        let bytes = response.into_bytes().await.map_err(|e| e.to_string())?;
        let value = json::parse(&bytes).map_err(|e| format!("graphql HTTP {status}: {e}"))?;
        let Value::Object(mut top) = value else {
            return Err(format!("graphql HTTP {status}: not an object"));
        };
        if let Some(errors) = top.get("errors") {
            let text = json::encode(errors, Layout::Compact).unwrap_or_default();
            return Err(format!(
                "graphql errors: {}",
                String::from_utf8_lossy(&text)
            ));
        }
        let data = top.remove("data").ok_or("graphql response has no data")?;
        if let Some(Value::Object(rate)) = get(&data, "rateLimit") {
            if let Some(cost) = rate.get("cost").and_then(as_i64) {
                self.counters
                    .graphql_cost
                    .fetch_add(cost.max(0) as u64, Ordering::Relaxed);
            }
            if let Some(left) = rate.get("remaining").and_then(as_i64) {
                self.counters
                    .graphql_remaining
                    .store(left, Ordering::Relaxed);
            }
        }
        Ok(data)
    }
}

/// One query for every PR in the batch, each under its own alias, selecting
/// the union of the fields its callers asked for.
pub fn build_query(owner: &str, repo: &str, prs: &BTreeMap<u64, BTreeSet<String>>) -> String {
    let mut q = format!("query{{r:repository(owner:\"{owner}\",name:\"{repo}\"){{");
    for (number, fields) in prs {
        let selection: Vec<&str> = fields.iter().map(String::as_str).collect();
        q.push_str(&format!(
            "p{number}:pullRequest(number:{number}){{{}}}",
            selection.join(" ")
        ));
    }
    q.push_str("}rateLimit{cost remaining}}");
    q
}

/// Hand each PR in the batch its own object back.
pub fn split(
    data: &Value,
    prs: &BTreeMap<u64, BTreeSet<String>>,
) -> Result<HashMap<u64, Fields>, String> {
    let repo = get(data, "r").ok_or("repository not found")?;
    let mut out = HashMap::new();
    for number in prs.keys() {
        if let Some(Value::Object(fields)) = get(repo, &format!("p{number}")) {
            out.insert(*number, fields.clone());
        }
    }
    Ok(out)
}

fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map.get(key),
        _ => None,
    }
}

fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Signed(n) => Some(*n),
        Value::Unsigned(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}

fn header_str(response: &http::Response, name: &str) -> Option<String> {
    response
        .header(name)
        .and_then(|v| std::str::from_utf8(v).ok())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn one_query_aliases_every_pr_with_its_union_of_fields() {
        let mut prs = BTreeMap::new();
        prs.insert(12, set(&["state", "title"]));
        prs.insert(15, set(&["mergeable"]));
        assert_eq!(
            build_query("o", "r", &prs),
            "query{r:repository(owner:\"o\",name:\"r\"){p12:pullRequest(number:12){state title}\
             p15:pullRequest(number:15){mergeable}}rateLimit{cost remaining}}"
        );
    }

    #[test]
    fn split_returns_each_alias_to_its_pr() {
        let data = json::parse(br#"{"r":{"p1":{"state":"OPEN"},"p2":{"state":"MERGED"}}}"#)
            .map_err(|e| e.to_string())
            .expect("test fixture");
        let mut prs = BTreeMap::new();
        prs.insert(1, set(&["state"]));
        prs.insert(2, set(&["state"]));
        let out = split(&data, &prs).expect("test fixture");
        assert_eq!(out[&2]["state"], Value::String("MERGED".into()));
        assert_eq!(out[&1]["state"], Value::String("OPEN".into()));
    }

    #[test]
    fn canceled_subscribers_cannot_grow_a_batch_past_its_member_cap() {
        let runtime = async_engine::RuntimeBuilder::multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.run(async {
            let identity =
                ghd_core::identity::Identity::from_token("github.com", "batch-fixture".into())
                    .expect("identity");
            let upstream = Arc::new(Upstream::new(&identity, Duration::ZERO).expect("upstream"));
            let (done, receiver) = async_engine::broadcast_channel(1).expect("channel");
            drop(receiver); // An abandoned waiting caller must not relax the cap.
            upstream.batches.lock().await.insert(
                ("o".into(), "r".into()),
                Batch {
                    prs: (1..=32).map(|number| (number, set(&["title"]))).collect(),
                    done,
                },
            );
            let error = async_engine::timeout(
                Duration::from_secs(1),
                upstream.pull_request("o", "r", 33, &set(&["title"])),
            )
            .await
            .expect("bounded refusal")
            .expect_err("bounded batch");
            assert!(error.contains("member capacity"));
            assert_eq!(
                upstream.batches.lock().await[&("o".into(), "r".into())]
                    .prs
                    .len(),
                32
            );
        });
    }
}
