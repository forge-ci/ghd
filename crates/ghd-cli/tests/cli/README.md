# `crates/ghd-cli/tests/cli/`

`ghd-cli`'s one integration-test target: `tests/cli/main.rs`, which
Cargo's autodiscovery names `cli` (the `tests/<name>/main.rs` layout —
see `crates/ghd/tests/api/README.md` for why it is preferred over a
flat `tests/cli.rs`). This is `ghd-cli:test:cli` in `ci.toml`'s
`[rust.tests].binaries`.

## Why this runs the real binary, not library code

`ghd-cli`'s `[[bin]] test = false` — there is no in-process unit
harness for the bin target. This is deliberate: the point of `ghd-cli`
is the shipped `ghd` executable, and the only thing worth proving is
"the compiled binary, run as a subprocess, behaves correctly" — the
same contract `tests/test_cli.py` (Python side) checks against the
*installed wheel's* binary.

## How it works

Every test resolves `CARGO_BIN_EXE_ghd` with `std::env::var` at
**runtime** and spawns it via `std::process::Command`, then asserts on
exit status, stdout and stderr. Runtime lookup, not `env!`: Cargo only
sets `CARGO_BIN_EXE_*` for real `cargo test`/`cargo build` runs, and
the Dylint lane runs `cargo check --workspace --all-targets`, where
`env!` would be a compile error and `option_env!` trips
`clippy::option_env_unwrap`. The module doc in `main.rs` has the full
reasoning.

## What's tested here

- `prints_a_nonempty_banner_by_default` — no arguments: exits 0 with
  non-empty stdout (the usage text).
- `version_flag_succeeds_with_nonempty_output` — `--version` exits 0
  and prints the `ghd <version> (<os>)` banner.
- `help_lists_the_subcommands` — `--help` mentions `pr view`, `stats`
  and `daemon`.
- `pr_view_without_a_number_fails_before_any_network_call` —
  `pr view -R o/r --json state` exits non-zero with `missing PR number`
  on stderr. Argument parsing fails before the client is called, so no
  daemon is started and GitHub is never contacted.

None of these tests need a GitHub token or a running daemon.

## Relationship to the Python-side CLI test

`tests/test_cli.py` (repo root) checks the `--version` contract
against the `ghd` binary resolved from `PATH` after a real wheel
install. This target checks the binary Cargo just built. Both need to
pass — one proves the Rust binary's own behavior, the other proves
packaging didn't break on the way to a wheel.

```bash
soldr cargo test -p ghd-cli --locked
```

## Adding a test here

Add the assertion to `main.rs` directly, and keep it offline (fail
before the network, as the `pr view` test does). If `ghd` grows new
subcommands, also update `action.yml`'s smoke test
(`action/smoke_test.py`) and `crates/ghd-cli/README.md`'s "Surface
contract" section — `ci/gates/action_surface.py` checks that
`action.yml` only references subcommands the binary actually exposes.
