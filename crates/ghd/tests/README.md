# `crates/ghd/tests/`

Integration tests for the `ghd` amalgam. There is exactly one target:

```
tests/
└── api/
    ├── main.rs     # ghd:test:api
    └── README.md   # what it checks and how to run both ship sets
```

## Why only one target

Each integration-test directory is its own link target (its own test
binary), and `ci.toml`'s `[rust.tests]` declares and caps every binary
(`binaries = [...]`, `max-binary`, `max-total`). An undeclared target is
a precheck finding. One target for the whole public surface keeps the
binary count, link time and disk use small; test *functions* inside it
are free.

## Why the amalgam is tested here and not in `src/`

`crates/ghd/Cargo.toml` sets `[lib] test = false`: the amalgam is only
`pub use` re-exports, and each re-exported item is unit-tested once in
the private crate that owns it (`ghd-core:lib`, `ghd-daemon:lib`,
`ghd-platform:lib`). What is left to prove is that a consumer of the
published crate actually sees those names, under each feature set it
can be built with. That is an external-consumer question, so it lives
in `tests/`, which links against `ghd` exactly as a downstream crate
would.

## Feature sets

`ci.toml`'s `[rust] ship = [[], ["daemon"]]`. The target is run once
per set; tests that need `run_daemon` are gated with
`#[cfg(feature = "daemon")]`.

```bash
soldr cargo test -p ghd --locked                      # []
soldr cargo test -p ghd --locked --features daemon    # ["daemon"]
```

Run `-p ghd` on its own for the `[]` set: a workspace-wide test run
unifies features, and `ghd-cli` turns `daemon` on.

## Ground rules

- Offline only. Nothing here may start a daemon, bind a port, or reach
  GitHub; `run_daemon` is checked by type, never called.
- Use only `ghd::*` paths. Naming `ghd_core` or `ghd_daemon` would test
  the private crates, not the public surface.
- Add new checks to `api/main.rs` rather than a new directory, unless a
  new binary is truly needed — then declare it in `ci.toml` first.

## Related

- `crates/ghd/README.md` — the amalgam and its feature.
- `crates/ghd-cli/tests/cli/` — the same idea for the shipped binary.
- `tests/` at the repo root — Python tests against the built wheel.
