# `crates/ghd-cli/tests/`

Integration tests for the `ghd` binary. There is exactly one target:

```
tests/
└── cli/
    ├── main.rs     # ghd-cli:test:cli
    └── README.md   # what it checks and why it uses CARGO_BIN_EXE_ghd
```

## Why a subprocess test

The deliverable of this crate is an executable, not a library. The bin
target has `test = false`, so there is no in-process harness; instead
`cli/main.rs` spawns the binary Cargo just built (`CARGO_BIN_EXE_ghd`,
read at runtime) and checks exit codes and output. That proves the
artifact itself: argv handling, the `main` dispatch, the exit-code
policy, and the text users and `action.yml` actually see.

## Why only one target

Every `tests/<name>/` is its own link target, and `ci.toml`'s
`[rust.tests].binaries` declares each one (undeclared targets are a
precheck finding, and binary size is capped by `max-binary` /
`max-total`). One target with several test functions costs one link.

## What the suite covers

- No arguments and `--help` print the usage text and exit 0.
- `--version` prints the banner and exits 0.
- `--help` names the `pr view`, `stats` and `daemon` subcommands (the
  surface `action.yml` and `ci/gates/action_surface.py` depend on).
- `pr view` with no number fails with `missing PR number` before any
  network call.

## Ground rules

- Offline only. The tests must not need a GitHub token, start a daemon
  or touch `api.github.com`. Choose inputs that fail in argument
  parsing (as the `pr view` test does) when checking error paths.
- Assert on observable behavior (exit status, stdout, stderr), not on
  internals — internals are covered by `ghd-core:lib` and
  `ghd-daemon:lib`.

## Running

```bash
soldr cargo test -p ghd-cli --locked
```

`cargo check` alone does not set `CARGO_BIN_EXE_ghd`; the tests panic
with an explanatory message if run outside `cargo test`/`cargo nextest`.

## Related

- `tests/test_cli.py` at the repo root checks `ghd --version` on the
  binary installed from the wheel.
- `crates/ghd/tests/` covers the library surface the binary is built on.
