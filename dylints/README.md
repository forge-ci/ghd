# `dylints/`

The repository's Dylint lint libraries. Each subdirectory is its own crate
(and its own Cargo *workspace* — see below), wired into the main
workspace only through `Cargo.toml`'s `[workspace.metadata.dylint]
libraries` table, never through `[workspace] members`.

## Why a separate workspace per library

Dylint lint libraries compile against `rustc_private` — unstable,
internal compiler crates (`rustc_ast`, `rustc_lint`, `rustc_span`, …) only
available on a nightly toolchain with the `rustc-dev` and `llvm-tools`
components. The main template workspace is pinned to a **stable** channel
(`rust-toolchain.toml` at the repo root: `1.95.0`) because the code it
ships must build on the same toolchain a downstream consumer's `cargo
build` would use. A lint library therefore cannot be a member of that
workspace — it needs its own `rust-toolchain.toml` pinning a specific
dated nightly, and Cargo resolves toolchains per workspace, not per crate.

Each `dylints/<name>/` directory is consequently a **standalone** crate
with its own `[workspace]` table (making it its own workspace root) and
its own `rust-toolchain.toml`. `soldr cargo dylint` / `soldr lint rust
--cross-dylint` find it through `[workspace.metadata.dylint].libraries`
at the main workspace's `Cargo.toml`, build it under its own nightly, and
load the resulting `cdylib` into the Dylint driver that checks the main
(stable-channel) workspace's source. This is exactly how Dylint always
works — the library's toolchain and the checked workspace's toolchain are
never the same channel.

## Pinning the library's nightly correctly (read before bumping either toolchain)

A Dylint library only loads into a driver built for the **exact** nightly
declared in the library's own `rust-toolchain.toml`. `setup-soldr`'s
`dylint: true` mode resolves that nightly automatically, but it resolves
it from the **main workspace's** stable channel (`rust-toolchain.toml`'s
`1.95.0`), via `zackees/soldr-toolchain`'s published
`rust-nightly-versions.v1.json` catalogue (the "1.95" bucket's
`selected` nightly). Every library under `dylints/` MUST pin that exact
same nightly in its own `rust-toolchain.toml`, or the driver Soldr builds
for the main workspace won't load it (a silent skip, not a hard error, in
some Dylint versions — see `RUST-004`/`RUST-003`).

**When the root `rust-toolchain.toml` channel moves to a new Rust minor
version, every `dylints/*/rust-toolchain.toml` must be re-pinned to that
new channel's catalogued nightly in the same commit.** This repository
has one library today (`platform_boundary/`); check its `rust-toolchain.toml`
comment for how the current pin was verified.

## Libraries

| Library | What it bans | Docs |
|---|---|---|
| [`platform_boundary/`](./platform_boundary/README.md) | Host-platform `#[cfg]`/`#[cfg_attr]`/`cfg!`/`cfg_select!` and native platform paths (`std::os::*`, `libc::`, `windows_sys::`) outside `ghd-platform`'s selector file and `platforms/**` trees | its own README |

## Where this runs

The `dylint` job in `.github/workflows/ci.yml` (`ci/dylint.py`) — one
Linux job checks the host target **and** every declared cross target
(`ci.toml`'s `[platforms]`) using this same compiled library, per
[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) §5. It is
never invoked with bare `cargo`/`cargo-dylint` — always through
`soldr cargo dylint` / `soldr dylint prepare --target` (RUST-002).

## Adding a second library

1. `dylints/<name>/` as its own crate + `[workspace]` (copy
   `platform_boundary/Cargo.toml`'s shape: `crate-type = ["cdylib"]`,
   `dylint_linting`/`dylint_testing` dependencies, `.cargo/config.toml`'s
   `linker = "dylint-link"` rustflags).
2. Add `{ path = "dylints/<name>" }` to the root `Cargo.toml`'s
   `[workspace.metadata.dylint].libraries`.
3. Pin its `rust-toolchain.toml` to the SAME nightly every other library
   here uses (see above) — they all check the same workspace in the same
   driver invocation, so a mismatched nightly among them breaks the whole
   pass, not just the new library.
4. Give it `ui/` tests (`dylint_testing::ui_test`) and unit tests for any
   pure detection logic, mirroring `platform_boundary/`'s split between
   rustc-integration UI tests and plain `#[test]` string-scanning tests.
