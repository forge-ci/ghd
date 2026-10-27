//! `ghd-daemon`: the long-lived side of ghd.
//!
//! - [`server`]: loopback HTTP routes over a per-PR cache (`fresh` →
//!   `revalidated` via a free ETag 304 → `fetched`)
//! - [`upstream`]: GitHub itself: conditional REST sentinels and the
//!   batched, alias-merged GraphQL reader
//!
//! Built on kernal-api only: its async engine, HTTP client/server, JSON and
//! secure random. No crate here names tokio, reqwest or hyper.

pub(crate) use ghd_platform as platform;

mod gh_executor;
mod gh_reads;
mod gh_render;
mod governance;
pub mod graphql;
mod persistence;
mod rest_reads;
mod retention;
mod run_collection;
pub mod server;
pub mod upstream;

pub use server::run;
