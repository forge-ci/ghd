//! `ghd-core`: everything on the caller's side of the loopback daemon.
//!
//! - [`proto`]: the protobuf messages carried over loopback HTTP
//! - [`endpoint`]: where a running daemon listens, and its bearer token
//! - [`client`]: one std-only HTTP/1.1 call, starting the daemon if needed
//! - [`cli`]: the gh-compatible command surface (`ghd pr view`, `ghd stats`)
//! - [`fields`]: the `gh pr view --json` fields ghd can answer
//! - [`graphql`]: `ghd api graphql`, gh's `-f`/`-F` argument shape
//!
//! No async runtime and no TLS live here, so a warm `ghd` invocation costs
//! one loopback round trip. The daemon itself is `ghd-daemon`.
//!
//! Reaches the OS only through `crate::platform::*` (`ghd-platform`).

pub(crate) use ghd_platform as platform;

pub mod cli;
pub mod client;
pub mod coherence;
pub mod compat;
pub mod endpoint;
pub mod fields;
pub mod graphql;
pub mod identity;
pub mod operation;
pub mod proto;
pub mod reads;
pub mod resource;
pub mod rest;

/// `ghd <version> (<os>)`, printed by `ghd --version`.
pub fn version_banner() -> String {
    format!(
        "ghd {} ({})",
        env!("CARGO_PKG_VERSION"),
        platform::host::os_name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_banner_contains_the_crate_version() {
        assert!(version_banner().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn version_banner_contains_a_known_host_os() {
        let banner = version_banner();
        assert!(
            ["windows", "linux", "macos"]
                .iter()
                .any(|os| banner.contains(os)),
            "banner {banner:?} did not name a known host OS",
        );
    }
}
