//! Exercise actual loopback HTTP with concurrent callers and private caches.

use super::*;
use ghd_core::identity::Identity;
use kernal_api::async_engine;
use kernal_api::http::{self, Method};

type TestServer = (u16, Arc<State>, async_engine::Task<io::Result<()>>);

async fn cached_server(identity: &Identity, login: &str) -> TestServer {
    let mut state = new_state(identity, "session-token".into()).expect("state");
    // Cache-isolation fixtures must never expire into a real upstream call,
    // regardless of GHD_FRESH_MS or scheduling delays on a busy runner.
    state.fresh_for = Duration::MAX;
    state.journal = ghd_core::coherence::Journal::at(std::env::temp_dir().join(format!(
        "ghd-cache-only-{}-{}",
        std::process::id(),
        identity.key()
    )));
    Arc::get_mut(&mut state.upstream)
        .expect("unshared upstream")
        .disable_network_for_test();
    let state = Arc::new(state);
    let key = gql::body("{viewer{login}}", &[]).expect("query body");
    let answer = format!("{{\"data\":{{\"viewer\":{{\"login\":\"{login}\"}}}}}}");
    state.graphql.lock().await.insert(
        key,
        Arc::new(Mutex::new(Some((answer.into_bytes(), Instant::now())))),
    );
    let fields = BTreeMap::from([("title".into(), Value::String(login.into()))]);
    state.cache.lock().await.insert(
        ("o".into(), "r".into(), 42),
        Arc::new(Mutex::new(Some(Entry {
            etag: None,
            fields,
            validated: Instant::now(),
        }))),
    );
    let server = Server::bind(
        SocketAddr::from(([127, 0, 0, 1], 0)),
        http_server::Limits::default(),
    )
    .await
    .expect("bind");
    let port = server.local_addr().expect("address").port();
    let handler_state = Arc::clone(&state);
    let task = async_engine::launch(server.serve(move |request| {
        let state = Arc::clone(&handler_state);
        async move { route(&state, request).await }
    }));
    (port, state, task)
}

async fn read(port: u16, key: &str, pr: bool) -> (u16, Vec<u8>) {
    let (path, body) = if pr {
        (
            "/v1/pr/view",
            PrViewRequest {
                owner: "o".into(),
                repo: "r".into(),
                number: 42,
                fields: vec!["title".into()],
                ..Default::default()
            }
            .encode_to_vec(),
        )
    } else {
        (
            "/v1/graphql",
            GraphqlRequest {
                query: "{viewer{login}}".into(),
                ..Default::default()
            }
            .encode_to_vec(),
        )
    };
    let client = http::Client::new(http::Limits::default()).expect("client");
    let response = client
        .execute(http::Request {
            method: Method::Post,
            url: &format!("http://127.0.0.1:{port}{path}"),
            headers: &[
                ("Authorization", "Bearer session-token"),
                ("X-Ghd-Identity", key),
            ],
            body: &body,
        })
        .await
        .expect("response");
    (
        response.status(),
        response.into_bytes().await.expect("body"),
    )
}

#[test]
fn concurrent_credentials_cannot_share_loopback_cached_answers() {
    let runtime = RuntimeBuilder::multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.run(async {
        let a = Identity::from_token("github.com", "fixture-a".into()).expect("identity");
        let b = Identity::from_token("github.com", "fixture-b".into()).expect("identity");
        let c = Identity::from_token("git.example.com", "fixture-a".into()).expect("identity");
        let (port_a, state_a, task_a) = cached_server(&a, "a").await;
        let (port_b, state_b, task_b) = cached_server(&b, "b").await;
        let (port_c, state_c, task_c) = cached_server(&c, "c").await;
        // Even knowing the session bearer cannot cross the identity boundary.
        for pr in [true, false] {
            assert_eq!(read(port_a, b.key(), pr).await.0, 401);
            assert_eq!(read(port_b, a.key(), pr).await.0, 401);
            assert_eq!(read(port_c, a.key(), pr).await.0, 401);
        }
        let mut tasks = Vec::new();
        for _ in 0..8 {
            for (port, key, login) in [
                (port_a, a.key(), "a"),
                (port_b, b.key(), "b"),
                (port_c, c.key(), "c"),
            ] {
                for pr in [true, false] {
                    let key = key.to_string();
                    tasks.push(async_engine::launch(async move {
                        let (status, bytes) = read(port, &key, pr).await;
                        assert_eq!(status, 200);
                        if pr {
                            let reply = PrViewResponse::decode(bytes.as_slice()).expect("protobuf");
                            assert_eq!(reply.source, "fresh");
                            assert_eq!(
                                json::parse(&reply.json).expect("PR JSON"),
                                Value::Object(BTreeMap::from([(
                                    "title".into(),
                                    Value::String(login.into())
                                )]))
                            );
                        } else {
                            let reply =
                                GraphqlResponse::decode(bytes.as_slice()).expect("protobuf");
                            assert_eq!(reply.source, "fresh");
                            assert_eq!(
                                reply.json,
                                format!("{{\"data\":{{\"viewer\":{{\"login\":\"{login}\"}}}}}}")
                                    .as_bytes()
                            );
                        }
                    }));
                }
            }
        }
        for task in tasks {
            task.await.expect("reader task");
        }
        for state in [state_a, state_b, state_c] {
            assert_eq!(state.graphql_fresh.load(Ordering::Relaxed), 8);
            assert_eq!(state.fresh.load(Ordering::Relaxed), 8);
        }
        for task in [task_a, task_b, task_c] {
            task.cancel();
            let _ = task.await;
        }
    });
}
