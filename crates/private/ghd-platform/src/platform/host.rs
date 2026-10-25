//! Host-neutral machine-facts facade.
//!
//! Owns the one real capability this crate proves end to end: the running
//! host's OS name and its native executable suffix. Both functions defer
//! to [`crate::platform_imp::host`], the implementation `lib.rs` selected
//! with `cfg_select!`. Nothing in this file branches on `cfg`.

/// The running host's OS name.
///
/// Returns one of `"windows"`, `"linux"`, or `"macos"` — the same three
/// arms `lib.rs`'s `cfg_select!` compiles. There is no fourth value: a
/// host outside this set fails to compile the crate, not this function.
pub fn os_name() -> &'static str {
    crate::platform_imp::host::os_name()
}

/// The suffix the OS/Cargo appends to native executables on this host.
///
/// `".exe"` on Windows, empty everywhere else.
pub fn exe_suffix() -> &'static str {
    crate::platform_imp::host::exe_suffix()
}
