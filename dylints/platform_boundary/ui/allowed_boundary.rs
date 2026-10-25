// The boundary's inside: this fixture stands in for
// `ghd-platform/src/lib.rs` and `ghd-platform/src/platforms/**`
// (it is excluded from the lint's scope by filename, mirroring how those
// two paths are excluded by ci.toml's `[allow] platform-selector` /
// `platform-code`). Host cfg, `cfg_select!`, and native platform paths are
// all allowed here; feature/test cfg without a host selector is allowed
// everywhere in the workspace, boundary or not.

#[cfg(target_os = "linux")]
mod selected {
    pub fn f() -> u8 {
        1
    }
}

#[cfg(feature = "tokio-console")]
fn feature_gated() {}

#[cfg(test)]
mod tests {
    #[test]
    fn neutral() {
        assert_eq!(super::selected::f(), 1);
    }
}

fn main() {}
