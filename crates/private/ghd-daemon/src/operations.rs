//! Shutdown rejects active work; one lifetime owner lock prevents competing daemons.
use crate::server::State;
use kernal_api::{
    async_engine::{self, CancellationSource},
    http_server::Response,
    json::{self, Value},
};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
pub(crate) struct Lifecycle {
    state: Mutex<(bool, usize)>,
    pub(crate) shutdown: CancellationSource,
}
pub(crate) struct Active<'a>(&'a Lifecycle);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.1 -= 1;
        }
    }
}
impl Lifecycle {
    pub(crate) fn enter(&self) -> io::Result<Active<'_>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("lifecycle lock poisoned"))?;
        if state.0 {
            return Err(io::Error::other("daemon is stopping; retry after restart"));
        }
        state.1 += 1;
        Ok(Active(self))
    }
}
fn text(status: u16, body: &str) -> Response {
    Response::new(status, body.as_bytes().to_vec()).unwrap_or_default()
}
fn encoded(value: Value) -> Response {
    match json::encode(&value, json::Layout::Compact) {
        Ok(body) => Response::new(200, body).unwrap_or_default(),
        Err(e) => text(500, &e.to_string()),
    }
}
pub(crate) async fn stop(state: &State) -> Response {
    let Ok(mut lifecycle) = state.lifecycle.state.lock() else {
        return text(500, "lifecycle poisoned");
    };
    if lifecycle.0 {
        return text(200, "stopping");
    }
    if lifecycle.1 != 0
        || state.gh_permits.available_permits() != 4
        || state.read_permits.available_permits() != 32
        || !state.control.idle()
    {
        return text(
            409,
            "active requests or upstream workers; retry stop after they finish",
        );
    }
    lifecycle.0 = true;
    let lifecycle = Arc::clone(&state.lifecycle);
    async_engine::launch(async move {
        async_engine::sleep(Duration::from_millis(250)).await;
        lifecycle.shutdown.cancel();
    })
    .detach();
    text(200, "stopping")
}
pub(crate) async fn keys(state: &State) -> Response {
    let cache = state.retained.lock().await;
    let (entries, bytes) = cache.usage();
    encoded(Value::Object(std::collections::BTreeMap::from([
        (
            "keys".into(),
            Value::Array(
                cache
                    .ordered()
                    .map(|(key, _)| Value::String(key.into()))
                    .collect(),
            ),
        ),
        ("entries".into(), Value::Unsigned(entries as u64)),
        ("retained_bytes".into(), Value::Unsigned(bytes as u64)),
    ])))
}
pub(crate) async fn flush(state: &State) -> Response {
    // Advancing the shared epoch invalidates in-flight publishers and restart bodies.
    let journal = state.journal.clone();
    match async_engine::launch_blocking(move || journal.begin()?.finish()).await {
        Ok(Ok(())) => {
            state.retained.lock().await.clear();
            text(200, "{\"flushed\":true}")
        }
        result => text(500, &format!("flush failed: {result:?}")),
    }
}
