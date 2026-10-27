//! `ghd`: the public API amalgam.
//!
//! This is the only crate in the workspace published to crates.io. It
//! carries **no logic of its own**: every symbol below is a `pub use`
//! re-export from a private crate under `crates/private/`, wired through
//! Cargo features. `ci.toml`'s `[rust] ship = [[], ["daemon"]]` names the
//! only two feature sets this crate is ever compiled or tested with; both
//! are exercised by `tests/api/main.rs`.
//!
//! - default: the client side (`ghd-core`): protocol, endpoint discovery,
//!   the loopback client and the gh-compatible CLI.
//! - `daemon`: adds `ghd-daemon`, the cache/batcher/GitHub side. The `ghd`
//!   binary needs it; a library caller that only talks to a running
//!   daemon does not.
//!
//! `#[cfg(feature = "daemon")]` only decides whether a name exists, never
//! what it does.

pub use ghd_core::{
    cache_policy, cli, client, coherence, compat, durable, endpoint, fields, governor,
    governor_ledger, identity, operation, proto, reads, resource, rest, retention, snapshot,
    version_banner,
};

#[cfg(feature = "daemon")]
pub use ghd_daemon::run as run_daemon;
