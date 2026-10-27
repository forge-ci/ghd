//! Restart data is retained only for routes with origin validators.

use crate::{
    gh_reads, rest_reads,
    server::{Entry, State},
};
use ghd_core::{durable::Store, proto::GhReadResponse};
use kernal_api::json::{self, Layout, Value};
use prost::Message;
use std::{io, sync::Arc, time::Duration};

#[derive(Clone, PartialEq, Message)]
struct Body {
    #[prost(bytes = "vec", tag = "1")]
    bytes: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    stderr: Vec<u8>,
    #[prost(string, tag = "3")]
    etag: String,
    #[prost(string, repeated, tag = "4")]
    fields: Vec<String>,
    #[prost(uint64, tag = "5")]
    ttl_ms: u64,
}

pub(crate) fn open(
    identity: &ghd_core::identity::Identity,
    journal: ghd_core::coherence::Journal,
) -> io::Result<Option<Arc<Store>>> {
    match ghd_core::config::value("GHD_CACHE_DISK")?.as_deref() {
        None | Some("0") => Ok(None),
        Some("1") => Ok(Some(Arc::new(Store::new(
            ghd_core::endpoint::directory(identity)?.join("cache-v1"),
            identity.key().into(),
            crate::retention::limit("GHD_CACHE_MAX_ENTRIES", 4096)?,
            crate::retention::limit("GHD_CACHE_MAX_BYTES", 64 * 1024 * 1024)?,
            journal,
        )))),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "GHD_CACHE_DISK must be 0 or 1",
        )),
    }
}

async fn load(state: &State, key: &str, epoch: u64) -> io::Result<Option<Body>> {
    if crate::policy::current().bypass {
        return Ok(None);
    }
    let Some(store) = &state.disk else {
        return Ok(None);
    };
    let store = Arc::clone(store);
    let key = key.to_owned();
    let bytes = kernal_api::async_engine::launch_blocking(move || store.get(&key, epoch))
        .await
        .map_err(io::Error::other)??;
    bytes
        .map(|bytes| Body::decode(bytes.as_slice()).map_err(io::Error::other))
        .transpose()
}

async fn save(state: &State, key: &str, epoch: u64, body: Body) -> io::Result<()> {
    if crate::policy::current().bypass {
        return Ok(());
    }
    let Some(store) = &state.disk else {
        return Ok(());
    };
    let store = Arc::clone(store);
    let key = key.to_owned();
    kernal_api::async_engine::launch_blocking(move || store.put(&key, body.encode_to_vec(), epoch))
        .await
        .map_err(io::Error::other)?
}

pub(crate) async fn forget(state: &State, key: &str, epoch: u64) -> io::Result<()> {
    let Some(store) = &state.disk else {
        return Ok(());
    };
    let store = Arc::clone(store);
    let key = key.to_owned();
    kernal_api::async_engine::launch_blocking(move || store.remove(&key, epoch))
        .await
        .map_err(io::Error::other)?
}

pub(crate) async fn restore_pr(state: &State, key: &str, epoch: u64) -> io::Result<Option<Entry>> {
    let Some(body) = load(state, key, epoch).await? else {
        return Ok(None);
    };
    let Value::Object(fields) = json::parse(&body.bytes).map_err(io::Error::other)? else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid persisted PR fields",
        ));
    };
    Ok(Some(Entry {
        etag: Some(body.etag),
        ttl: Duration::from_millis(body.ttl_ms),
        fields,
        validated: None,
    }))
}

pub(crate) async fn save_pr(state: &State, key: &str, epoch: u64, entry: &Entry) -> io::Result<()> {
    if state.disk.is_none() {
        return Ok(());
    }
    let Some(etag) = &entry.etag else {
        return forget(state, key, epoch).await;
    };
    let bytes = json::encode(&Value::Object(entry.fields.clone()), Layout::Compact)
        .map_err(io::Error::other)?;
    save(
        state,
        key,
        epoch,
        Body {
            bytes,
            etag: etag.clone(),
            ttl_ms: u64::try_from(entry.ttl.as_millis()).unwrap_or(u64::MAX),
            ..Default::default()
        },
    )
    .await
}

pub(crate) async fn restore_gh(
    state: &State,
    key: &str,
    epoch: u64,
) -> io::Result<Option<gh_reads::Entry>> {
    let Some(body) = load(state, key, epoch).await? else {
        return Ok(None);
    };
    Ok(Some(gh_reads::Entry {
        response: GhReadResponse::decode(body.bytes.as_slice()).map_err(io::Error::other)?,
        fields: body.fields,
        validated: None,
        etag: Some(body.etag),
        ttl: Duration::from_millis(body.ttl_ms),
    }))
}

pub(crate) async fn save_gh(
    state: &State,
    key: &str,
    epoch: u64,
    entry: &gh_reads::Entry,
) -> io::Result<()> {
    if state.disk.is_none() {
        return Ok(());
    }
    let Some(etag) = &entry.etag else {
        return forget(state, key, epoch).await;
    };
    save(
        state,
        key,
        epoch,
        Body {
            bytes: entry.response.encode_to_vec(),
            etag: etag.clone(),
            fields: entry.fields.clone(),
            ttl_ms: u64::try_from(entry.ttl.as_millis()).unwrap_or(u64::MAX),
            ..Default::default()
        },
    )
    .await
}

pub(crate) async fn restore_rest(
    state: &State,
    key: &str,
    epoch: u64,
) -> io::Result<Option<rest_reads::Entry>> {
    let Some(body) = load(state, key, epoch).await? else {
        return Ok(None);
    };
    Ok(Some(rest_reads::Entry {
        body: body.bytes,
        stderr: body.stderr,
        etag: Some(body.etag),
        ttl: Duration::from_millis(body.ttl_ms),
        validated: None,
    }))
}

pub(crate) async fn save_rest(
    state: &State,
    key: &str,
    epoch: u64,
    entry: &rest_reads::Entry,
) -> io::Result<()> {
    if state.disk.is_none() {
        return Ok(());
    }
    let Some(etag) = &entry.etag else {
        return forget(state, key, epoch).await;
    };
    save(
        state,
        key,
        epoch,
        Body {
            bytes: entry.body.clone(),
            stderr: entry.stderr.clone(),
            etag: etag.clone(),
            ttl_ms: u64::try_from(entry.ttl.as_millis()).unwrap_or(u64::MAX),
            ..Default::default()
        },
    )
    .await
}
