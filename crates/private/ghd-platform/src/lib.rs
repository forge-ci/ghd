//! The single place `ghd-platform` selects its host-platform
//! implementation.
//!
//! Every other crate in this workspace that needs to reach the OS aliases
//! this crate as `crate::platform` (`pub(crate) use ghd_platform as
//! platform;`) and calls only the neutral facade in [`host`]. No other
//! production source file in this workspace may choose the host with
//! `#[cfg(target_os = ...)]`, `#[cfg(windows)]`, `#[cfg(unix)]`, or
//! `cfg!(...)`, and no other crate may name a concrete `platforms::*`
//! implementation module directly.
//!
//! The `cfg_select!` block below is the **only** host-selection site in
//! this workspace. There is deliberately no fallback arm and no `unix`
//! arm: a host OS this crate does not implement fails to compile here,
//! loudly, rather than silently falling back to a guess.
//!
//! Pattern and wording adapted from soldr's `crates/soldr-platform/src/
//! lib.rs` (zackees/soldr issues #2493, #2498) and kernal-api's
//! `docs/platform-boundary.md`. This crate implements one real capability
//! (`host::os_name` + `host::exe_suffix`) rather than soldr's full
//! process/fs/ipc/executable/host surface — enough to prove the boundary
//! shape without a second native SDK dependency in the template.

use std::cfg_select;

mod platform;

/// The cfg-free facade. See [`platform::host`] for the one capability this
/// crate exposes.
pub use platform::{fs, host, process};

cfg_select! {
    target_os = "windows" => {
        #[path = "platforms/windows/mod.rs"]
        mod platform_windows;
        pub(crate) use platform_windows as platform_imp;
    },
    target_os = "linux" => {
        #[path = "platforms/linux/mod.rs"]
        mod platform_linux;
        pub(crate) use platform_linux as platform_imp;
    },
    target_os = "macos" => {
        #[path = "platforms/macos/mod.rs"]
        mod platform_macos;
        pub(crate) use platform_macos as platform_imp;
    },
}

#[cfg(test)]
mod tests {
    use super::host;

    // These checks go through the facade, never `cfg`: they assert what
    // the running binary actually observes, not what target it was told
    // to compile for. That is the whole point of a runtime-checked
    // facade — see the module doc above and #6's "duplicate across trees
    // rather than add a selector" rule.

    #[test]
    fn os_name_is_one_of_the_three_supported_hosts() {
        let name = host::os_name();
        assert!(
            ["windows", "linux", "macos"].contains(&name),
            "unexpected host::os_name(): {name}",
        );
    }

    #[test]
    fn exe_suffix_matches_the_standard_librarys_own_answer() {
        // `std::env::consts::EXE_SUFFIX` is the standard library's own
        // runtime-visible constant for this same fact. Comparing against
        // it (rather than a literal per branch) keeps this test from
        // silently agreeing with itself if `platforms/<os>/host.rs` ever
        // drifts from Rust's own convention.
        assert_eq!(host::exe_suffix(), std::env::consts::EXE_SUFFIX);
    }

    #[test]
    fn os_name_and_exe_suffix_agree_on_windows_ness() {
        let is_windows_name = host::os_name() == "windows";
        let is_windows_suffix = !host::exe_suffix().is_empty();
        assert_eq!(is_windows_name, is_windows_suffix);
    }
}
