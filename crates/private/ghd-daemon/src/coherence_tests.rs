//! #8: controlled loopback schedules for writes overlapping cached/in-flight reads.

use super::*;
use ghd_core::{coherence::Journal, identity::Identity};
use kernal_api::async_engine::{self, Notify};
use kernal_api::http::{self, Method};
use std::path::PathBuf;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Harness {
    state: Arc<State>,
    port: u16,
    key: String,
    directory: PathBuf,
    started: Arc<Notify>,
    release: Arc<Notify>,
    tasks: Vec<async_engine::Task<io::Result<()>>>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.tasks.clear();
        if self.directory.exists() {
            std::fs::remove_dir_all(&self.directory).expect("clean coherence fixture");
        }
    }
}

async fn mock_upstream(
    started: Arc<Notify>,
    release: Arc<Notify>,
) -> (u16, async_engine::Task<io::Result<()>>) {
    let upstream = Server::bind(
        SocketAddr::from(([127, 0, 0, 1], 0)),
        http_server::Limits::default(),
    )
    .await
    .expect("upstream");
    let upstream_port = upstream.local_addr().expect("port").port();
    let version = Arc::new(AtomicU64::new(0));
    let reads = Arc::new(AtomicU64::new(0));
    let handler_started = Arc::clone(&started);
    let handler_release = Arc::clone(&release);
    let upstream_task = async_engine::launch(upstream.serve(move |request| {
        let version = Arc::clone(&version);
        let reads = Arc::clone(&reads);
        let started = Arc::clone(&handler_started);
        let release = Arc::clone(&handler_release);
        async move {
            let mutation = String::from_utf8_lossy(request.body()).contains("mutation");
            let before = version.load(Ordering::Relaxed);
            if mutation {
                version.fetch_add(1, Ordering::Relaxed);
            } else if reads.fetch_add(1, Ordering::Relaxed) == 0 {
                started.notify_one();
                release.notified().await;
            }
            text(200, &format!("{{\"data\":{{\"version\":{before}}}}}"))
        }
    }));
    (upstream_port, upstream_task)
}

impl Harness {
    async fn start(disabled: bool) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "ghd-coherence-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        assert!(!directory.exists(), "fixture owns a new directory");
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (upstream_port, upstream_task) =
            mock_upstream(Arc::clone(&started), Arc::clone(&release)).await;
        let identity =
            Identity::from_token("github.com", "fixture-coherence".into()).expect("identity");
        let mut state = new_state(&identity, "session".into()).expect("state");
        state.journal = Journal::at(directory.clone());
        state.fresh_for = Duration::MAX;
        let upstream = Arc::get_mut(&mut state.upstream).expect("unshared");
        if disabled {
            upstream.disable_network_for_test();
        } else {
            upstream.set_endpoint_for_test(&format!("http://127.0.0.1:{upstream_port}"));
        }
        let state = Arc::new(state);
        let daemon = Server::bind(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            http_server::Limits::default(),
        )
        .await
        .expect("daemon");
        let port = daemon.local_addr().expect("port").port();
        let handler = Arc::clone(&state);
        let daemon_task = async_engine::launch(daemon.serve(move |request| {
            let state = Arc::clone(&handler);
            async move { route(&state, request).await }
        }));
        Self {
            state,
            port,
            key: identity.key().into(),
            directory,
            started,
            release,
            tasks: vec![daemon_task, upstream_task],
        }
    }
}

async fn request(port: u16, key: &str, query: &str) -> (u16, GraphqlResponse) {
    let body = GraphqlRequest {
        query: query.into(),
        ..Default::default()
    }
    .encode_to_vec();
    let client = http::Client::new(http::Limits::default()).expect("client");
    let response = client
        .execute(http::Request {
            method: Method::Post,
            url: &format!("http://127.0.0.1:{port}/v1/graphql"),
            headers: &[("Authorization", "Bearer session"), ("X-Ghd-Identity", key)],
            body: &body,
        })
        .await
        .expect("response");
    let status = response.status();
    let bytes = response.into_bytes().await.expect("bytes");
    (
        status,
        GraphqlResponse::decode(bytes.as_slice()).expect("protobuf"),
    )
}

#[test]
fn mutation_clears_both_cache_tiers_and_rejects_an_old_inflight_result() {
    let runtime = RuntimeBuilder::multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.run(async {
        let harness = Harness::start(false).await;
        harness
            .state
            .cache
            .lock()
            .await
            .insert(("o".into(), "r".into(), 1), Arc::new(Mutex::new(None)));
        harness
            .state
            .gh_reads
            .lock()
            .await
            .insert("resource".into(), Arc::new(Mutex::new(None)));
        harness
            .state
            .rest_reads
            .lock()
            .await
            .insert("rest".into(), Arc::new(Mutex::new(None)));
        let port = harness.port;
        let key = harness.key.clone();
        let pending = async_engine::launch(async move { request(port, &key, "{ version }").await });
        async_engine::timeout(Duration::from_secs(5), harness.started.notified())
            .await
            .expect("upstream read started");
        let (status, reply) = request(
            harness.port,
            &harness.key,
            "fragment F on User { id } mutation Write { updateUser { ...F } }",
        )
        .await;
        assert_eq!(status, 200);
        assert!(reply.error.is_empty());
        assert!(
            harness.state.cache.lock().await.is_empty(),
            "PR cache invalidated too"
        );
        assert!(harness.state.graphql.lock().await.is_empty());
        assert!(harness.state.gh_reads.lock().await.is_empty());
        assert!(harness.state.rest_reads.lock().await.is_empty());
        harness.release.notify_one();
        let (status, stale) = pending.await.expect("pending reader");
        assert_eq!(status, 409);
        assert!(
            stale.json.is_empty(),
            "old in-flight body cannot be returned as current"
        );
        let (status, fresh) = request(harness.port, &harness.key, "{ version }").await;
        assert_eq!(status, 200);
        assert_eq!(fresh.json, br#"{"data":{"version":1}}"#);
        assert_eq!(fresh.source, "fetched");
        let (_, cached) = request(harness.port, &harness.key, "{ version }").await;
        assert_eq!(cached.json, fresh.json);
        assert_eq!(cached.source, "fresh");
    });
}

#[test]
fn uncertain_mutation_leaves_a_restart_visible_fence() {
    let runtime = RuntimeBuilder::multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.run(async {
        let harness = Harness::start(true).await;
        let (status, failure) = request(
            harness.port,
            &harness.key,
            "mutation Write { updateUser { id } }",
        )
        .await;
        assert_eq!(status, 502);
        assert!(!failure.error.is_empty());
        assert!(
            Journal::at(harness.directory.clone()).epoch().is_err(),
            "restart cannot forget the uncertainty"
        );
        let (status, read) = request(harness.port, &harness.key, "{ version }").await;
        assert_eq!(status, 409);
        assert!(read.json.is_empty());
    });
}
