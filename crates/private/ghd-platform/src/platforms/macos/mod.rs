//! macos host implementation index.
//!
//! Compiles only when `lib.rs`'s `cfg_select!` picks the `target_os = "macos"`
//! arm. It is the only place in this crate allowed to assume that host.

pub(crate) mod fs;
pub(crate) mod host;
pub(crate) mod process;
