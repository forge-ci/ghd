//! One budget for all cache namespaces, including empty coalescing slots.

use crate::{
    gh_reads, rest_reads,
    server::{Entry, GraphqlSlot, Slot, State},
};
use kernal_api::async_engine::Mutex;
use std::{io, sync::Arc};

#[derive(Clone)]
pub(crate) enum CachedSlot {
    Pr(Slot),
    Graphql(GraphqlSlot),
    Gh(Arc<Mutex<Option<gh_reads::Entry>>>),
    Rest(Arc<Mutex<Option<rest_reads::Entry>>>),
    Snapshot(Arc<Mutex<Option<crate::snapshots::Entry>>>),
}

impl CachedSlot {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Pr(a), Self::Pr(b)) => Arc::ptr_eq(a, b),
            (Self::Graphql(a), Self::Graphql(b)) => Arc::ptr_eq(a, b),
            (Self::Gh(a), Self::Gh(b)) => Arc::ptr_eq(a, b),
            (Self::Rest(a), Self::Rest(b)) => Arc::ptr_eq(a, b),
            (Self::Snapshot(a), Self::Snapshot(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

pub(crate) type Cache = Mutex<ghd_core::retention::Cache<CachedSlot>>;

pub(crate) fn limit(name: &str, default: usize) -> io::Result<usize> {
    match std::env::var(name) {
        Ok(value) => value.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} must be a nonnegative integer"),
            )
        }),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    }
}

pub(crate) fn new() -> io::Result<Cache> {
    Ok(Mutex::new(ghd_core::retention::Cache::new(
        limit("GHD_CACHE_MAX_ENTRIES", 4096)?,
        limit("GHD_CACHE_MAX_BYTES", 64 * 1024 * 1024)?,
    )))
}

pub(crate) fn key(namespace: &str, body: &[u8]) -> String {
    format!(
        "{namespace}:{}",
        kernal_api::hash::blake3_bytes(body).to_hex()
    )
}

pub(crate) async fn slot(state: &State, key: &str, candidate: CachedSlot) -> CachedSlot {
    let mut cache = state.retained.lock().await;
    if let Some(slot) = cache.get(key) {
        return slot.clone();
    }
    cache.insert(key.into(), candidate.clone(), 1024 + key.len() * 3);
    candidate
}

/// Reserve bytes before publishing. An evicted reader cannot recreate its slot.
/// Active owners of detached slots are bounded by the request admission permits.
pub(crate) async fn reserve(state: &State, key: &str, slot: CachedSlot, bytes: usize) -> bool {
    let mut cache = state.retained.lock().await;
    if !cache.get(key).is_some_and(|current| current.same(&slot)) {
        return false;
    }
    cache.insert(key.into(), slot, bytes.saturating_add(1024 + key.len() * 3))
}

pub(crate) async fn pr(state: &State, key: &str) -> Slot {
    match slot(
        state,
        key,
        CachedSlot::Pr(Arc::new(Mutex::new(None::<Entry>))),
    )
    .await
    {
        CachedSlot::Pr(slot) => slot,
        _ => unreachable!("cache namespace"),
    }
}

pub(crate) async fn graphql(state: &State, key: &str) -> GraphqlSlot {
    match slot(state, key, CachedSlot::Graphql(Arc::default())).await {
        CachedSlot::Graphql(slot) => slot,
        _ => unreachable!("cache namespace"),
    }
}

pub(crate) async fn gh(state: &State, key: &str) -> Arc<Mutex<Option<gh_reads::Entry>>> {
    match slot(state, key, CachedSlot::Gh(Arc::default())).await {
        CachedSlot::Gh(slot) => slot,
        _ => unreachable!("cache namespace"),
    }
}

pub(crate) async fn rest(state: &State, key: &str) -> Arc<Mutex<Option<rest_reads::Entry>>> {
    match slot(state, key, CachedSlot::Rest(Arc::default())).await {
        CachedSlot::Rest(slot) => slot,
        _ => unreachable!("cache namespace"),
    }
}

pub(crate) async fn admit(state: &State) -> io::Result<kernal_api::async_engine::SemaphorePermit> {
    kernal_api::async_engine::timeout(
        std::time::Duration::from_secs(1),
        state.read_permits.acquire(),
    )
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            "cache read capacity is busy; retry",
        )
    })
}

pub(crate) async fn snapshot(
    state: &State,
    key: &str,
) -> Arc<Mutex<Option<crate::snapshots::Entry>>> {
    match slot(state, key, CachedSlot::Snapshot(Arc::default())).await {
        CachedSlot::Snapshot(slot) => slot,
        _ => unreachable!("cache namespace"),
    }
}
