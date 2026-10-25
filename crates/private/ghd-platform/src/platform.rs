//! Host-neutral capability facade.
//!
//! This file and everything under `platform/` is a facade surface: it may
//! re-export or wrap the selected concrete implementation in
//! [`crate::platform_imp`], but it contains **no host `cfg` of its own**.
//! `lib.rs` selects `platform_imp` exactly once with `cfg_select!`; this
//! module only names the leaves it re-exports.
//!
//! - [`host`]: the running host's OS name and native executable suffix
//! - [`fs`]: the per-user state directory and owner-private file writes
//! - [`process`]: launching the daemon detached from its caller

pub mod fs;
pub mod host;
pub mod process;
