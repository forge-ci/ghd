# `crates/ghd/tests/api/`

`ghd`'s one integration-test target: `tests/api/main.rs`, which
Cargo's autodiscovery names `api` (the `tests/<name>/main.rs` layout,
same convention as `examples/<name>/main.rs` and
`benches/<name>/main.rs` — used here instead of a flat `tests/api.rs`
so future helper code has somewhere to live without cluttering
`tests/`). This is `ghd:test:api` in `ci.toml`'s
`[rust.tests].binaries`.

## Why this is the amalgam's only test target

`ghd` itself has `[lib] test = false` — no unit-test harness, because
it carries no logic (see `crates/ghd/README.md`). Every symbol it
exports is already unit-tested once, in the private crate that owns
it. What this target proves instead is the **public API surface**:
that the re-exports exist, forward to the right implementation, and
compile under both of `ci.toml`'s `[rust] ship` feature sets (`[]` and
`["daemon"]`).

## What's tested here

- `version_banner_is_reexported_and_names_ghd` — `ghd::version_banner()`
  starts with `"ghd "` (proves the `ghd-core` → `ghd-platform` chain
  reaches a value through the amalgam).
- `client_surface_is_reexported` — `ghd::fields::PR_FIELDS` contains
  `"state"` and `ghd::endpoint::parse("1 t")` succeeds.
- Under `--features daemon` only (`#[cfg(feature = "daemon")]`):
  `daemon_feature_reexports_run_daemon` — `ghd::run_daemon` exists with
  type `fn() -> std::io::Result<()>`. It is never called: running it
  would bind a port and need a GitHub token.

No test here touches the network or starts a daemon.

## Running both ship sets

```bash
soldr cargo test -p ghd --locked                      # [] — 2 tests
soldr cargo test -p ghd --locked --features daemon    # ["daemon"] — 3 tests
```

A plain `soldr cargo test --workspace --locked` also runs this target
with `daemon` enabled, because `ghd-cli` depends on `ghd` with
`features = ["daemon"]` and Cargo unifies features across a single
invocation. To exercise the `[]` set specifically — which is what
proves the feature is optional, not just present — invoke `-p ghd`
alone, as above.

## Adding a test here

Add the assertion to `main.rs` directly (single-file target; split
into `tests/api/<helper>.rs` + `mod` only if this genuinely outgrows
one file). Gate anything that depends on an optional feature with the
matching `#[cfg(feature = "...")]`, and update the commands above if a
new feature is added to `ghd`.
