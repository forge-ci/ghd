# `ghd-cli`

The crate that builds the `ghd` binary shipped with the Python package
(`[[bin]] name = "ghd"`). Depends on the public `ghd` amalgam with the
`daemon` ship feature set (`ci.toml`'s `[rust] ship = [[], ["daemon"]]`)
— never on `ghd-core`/`ghd-daemon` directly (see `crates/README.md`'s
dependency-direction diagram).

Built via `soldr cargo build -p ghd-cli` for local iteration; for the
shipped artifact, the soldr PEP 517 backend builds it and bundles it
into the wheel directly (`[tool.soldr.pep517] bundle-bins = [{ bin =
"ghd", package = "ghd-cli" }]` in `pyproject.toml`) — there is no
post-build injection step (the former `ci/build_wheel.py`, removed in
[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) round
1). It lands at `<name>-<ver>.data/scripts/ghd[.exe]` inside the wheel;
pip extracts that straight into the venv's `Scripts/` (Win) or `bin/`
(POSIX) on install — no Python launcher in front of it.

## Responsibilities

`src/ghd.rs` is a dispatcher and nothing more:

- `ghd daemon` → `ghd::run_daemon()` (the long-lived loopback server,
  only present because this crate enables the `daemon` feature).
- anything else → `ghd::cli::run(&args)`: `pr view`, `stats`,
  `--version`, `--help` (no arguments also prints the usage text).
- exit-code policy: `0` on success; on error prints `ghd: <error>` to
  stderr and exits `1`.

Everything else (argument parsing, the loopback client, auto-starting
the daemon, the cache) lives in the private crates, reached only
through `ghd`.

## Surface contract

The subcommands/flags this binary exposes are part of the **composite
action contract** — `action.yml` installs `ghd` and
`action/smoke_test.py` runs `ghd --version`/`--help`. Two CI gates
enforce this:

- `ci/gates/action_yaml.py` checks the action file's structure.
- `ci/gates/action_surface.py` checks that every subcommand referenced
  by `action.yml` shows up in `ghd --help`.

## Why the binary is packaged into the wheel

A Python user installing the `ghd` package gets the `ghd` binary on
PATH. They don't need to install Rust, and they don't need a separate
distribution channel for the binary. The wheel is one artifact for both
deliverables (`_native` + `ghd`), staged by soldr's `bundle-bins`. The
binary matters beyond the CLI: when `ghd-py` (or any client) finds no
running daemon, `ghd-core`'s client spawns `ghd daemon` from `$GHD_EXE`,
the current executable, or `ghd` on PATH.

## Testing

`ghd-cli:test:cli` in `ci.toml`'s `[rust.tests].binaries` —
`tests/cli/main.rs`, described in `tests/cli/README.md`. `[[bin]] test
= false`: the binary target itself carries no unit-test harness; its
behavior is proven by running the actual compiled executable.

```bash
soldr cargo test -p ghd-cli --locked
```

## Cross-compilation

Every declared target in `ci.toml`'s `[platforms]` cross-compiles from
Linux through soldr: `soldr cargo check --workspace --locked --target
<triple>` (provisions target std automatically). No `rustup target
add`, no `ziglang`.

## Linting

Both `soldr cargo clippy --workspace --all-targets --locked -D
warnings` and `soldr cargo fmt --all -- --check` run via `./ci.py
clippy` and `./ci.py fmt`. Failing either fails the gate; no
`#[allow(...)]` without a justification comment.
