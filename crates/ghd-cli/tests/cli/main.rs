//! Integration test for the `ghd-cli` binary.
//!
//! Runs the actual compiled executable via
//! `CARGO_BIN_EXE_ghd` rather than calling library code
//! in-process, so this proves the *shipped* binary's behavior — the same
//! contract `tests/test_cli.py` checks against the installed wheel. This
//! is `crates/ghd-cli`'s one integration-test link target
//! (`ghd-cli:test:cli` in `ci.toml`'s `[rust.tests].binaries`).
//!
//! `std::env::var` at RUNTIME, not the compile-time `env!`/`option_env!`
//! macros: Cargo only populates `CARGO_BIN_EXE_*` for an actual `cargo
//! test`/`cargo build` run, never for `cargo check` (no binary is
//! linked -- Cargo's own E0463 help text on this crate says exactly
//! this: "Cargo sets build script variables at run time. Use
//! `std::env::var(...)` instead"). The Dylint lane's pre-expansion /
//! type-check passes run `cargo check --workspace --all-targets`
//! (zackees/ci.yml#6 round 2 §5b) specifically so it can lint every
//! test target too; `env!` would make that check-only pass a hard
//! compile error here even though nothing ever tries to spawn the
//! binary. `option_env!` avoids that compile error but trips
//! `clippy::option_env_unwrap` (deny by default) for the same reason
//! clippy bans it: unwrapping a compile-time-resolved `Option` reads as
//! "should have been `env!`". A genuine runtime `std::env::var` lookup
//! satisfies both: no compile-time macro for `cargo check` to fail on,
//! and clippy has no rule against it. The runtime panic still fires
//! exactly like `env!`'s compile error would, for anyone who actually
//! runs this test outside `cargo test`/`cargo nextest run`.

use std::process::Command;

// `allow-expect-in-tests` (clippy.toml) only covers `#[test]`-annotated
// functions / `#[cfg(test)]` modules; this helper is called BY tests but
// isn't itself one, so `expect_used` (Cargo.toml workspace lint,
// ci.yml#6 round M2-4 / ghd#13) still fires under
// the clippy gate's `-D warnings`. Both `.expect()`s below document a
// genuine test-only invariant (see the module doc comment above), so a
// scoped allow is correct here rather than restructuring to `?`.
#[allow(clippy::expect_used)]
fn run(args: &[&str]) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_ghd")
        .expect("CARGO_BIN_EXE_ghd not set -- run via `cargo test`, not `cargo check`");
    Command::new(bin)
        .args(args)
        .output()
        .expect("failed to spawn ghd")
}

#[test]
fn prints_a_nonempty_banner_by_default() {
    let output = run(&[]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.trim().is_empty());
}

#[test]
fn version_flag_succeeds_with_nonempty_output() {
    let output = run(&["--version"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.trim().is_empty());
}

#[test]
fn help_lists_the_subcommands() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for sub in ["gh <args...>", "pr view", "stats", "daemon"] {
        assert!(stdout.contains(sub), "--help is missing {sub:?}: {stdout}");
    }
}

#[test]
fn pr_view_without_a_number_fails_before_any_network_call() {
    let output = run(&["pr", "view", "-R", "o/r", "--json", "state"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing PR number"));
}

#[cfg(test)]
mod passthrough_tests {
    use super::*;

    struct FixtureState(std::path::PathBuf);

    impl Drop for FixtureState {
        fn drop(&mut self) {
            if self.0.exists() {
                std::fs::remove_dir_all(&self.0).expect("clean fixture state");
            }
        }
    }

    #[test]
    fn passthrough_fixture() {
        if std::env::var_os("GHD_FIXTURE").is_none() {
            return;
        }
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).expect("stdin");
        println!("FIXTURE_STDIN={input}");
        println!(
            "FIXTURE_ARGS={:?}",
            std::env::args().skip(1).collect::<Vec<_>>()
        );
        eprintln!("fixture-stderr");
        std::process::exit(7);
    }

    #[test]
    fn unbrokered_commands_do_not_resolve_credentials() {
        let root =
            std::env::temp_dir().join(format!("ghd-local-command-test-{}", std::process::id()));
        assert!(!root.exists());
        let _cleanup = FixtureState(root.clone());
        let bin = std::env::var("CARGO_BIN_EXE_ghd").expect("binary");
        let fixture = std::env::current_exe().expect("fixture executable");
        for args in [["auth", "status"], ["config", "list"]] {
            let output = Command::new(&bin)
                .arg("gh")
                .args(args)
                .env("GHD_GH_PATH", &fixture)
                .env("GHD_STATE_DIR", &root)
                .env_remove("GH_TOKEN")
                .env_remove("GITHUB_TOKEN")
                .env_remove("GH_ENTERPRISE_TOKEN")
                .env_remove("GITHUB_ENTERPRISE_TOKEN")
                .output()
                .expect("local passthrough");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            // The real-gh fixture is this executable's test harness. Its
            // output proves execution reached it without daemon discovery.
            assert!(String::from_utf8_lossy(&output.stdout).contains("test result: ok"));
            assert!(
                !root.exists(),
                "local commands cannot create a daemon or write journal"
            );
        }
    }

    #[test]
    fn passthrough_preserves_stdin_literal_args_stderr_and_exit_code() {
        use std::io::Write;
        let root =
            std::env::temp_dir().join(format!("ghd-passthrough-test-{}", std::process::id()));
        assert!(!root.exists(), "test owns a fresh directory");
        let _cleanup = FixtureState(root.clone());
        let bin = std::env::var("CARGO_BIN_EXE_ghd").expect("binary");
        let fixture = std::env::current_exe().expect("fixture executable");
        let mut child = Command::new(bin)
            .args([
                "gh",
                "--exact",
                "passthrough_tests::passthrough_fixture",
                "--nocapture",
                "--",
                "--jq",
                ".title",
                "--template",
                "{{ .title }}",
                "@-",
            ])
            .env("GHD_GH_PATH", fixture)
            .env("GHD_FIXTURE", "1")
            .env("GHD_STATE_DIR", &root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn ghd");
        child
            .stdin
            .take()
            .expect("stdin pipe")
            .write_all(b"piped-body")
            .expect("write stdin");
        let output = child.wait_with_output().expect("output");
        assert_eq!(output.status.code(), Some(7));
        assert!(
            ghd::coherence::Journal::at(root.join("coherence-v1"))
                .epoch()
                .is_err(),
            "a nonzero writer may have partially succeeded and must remain fenced"
        );
        let stdout = String::from_utf8(output.stdout).expect("UTF8");
        assert!(stdout.contains("FIXTURE_STDIN=piped-body"));
        assert!(stdout.contains("--jq") && stdout.contains(".title"));
        assert!(stdout.contains("{{ .title }}") && stdout.contains("@-"));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr"),
            "fixture-stderr\n"
        );
    }
}
