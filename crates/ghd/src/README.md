# `crates/ghd/src/`

The source of the `ghd` amalgam: one file, `lib.rs`, and no logic.

## What's here

| File | Contents |
|---|---|
| `lib.rs` | Crate docs plus `pub use` re-exports from the private crates. |

That is the whole directory, on purpose. The amalgam is the published
face of the workspace; the code lives in `crates/private/*`, where each
crate is compiled and unit-tested exactly once.

## The re-exports

```rust
pub use ghd_core::{cli, client, endpoint, fields, proto, version_banner};

#[cfg(feature = "daemon")]
pub use ghd_daemon::run as run_daemon;
```

- Always present (client side, from `ghd-core`):
  - `cli` — the gh-compatible command surface (`cli::run`,
    `cli::pr_view_request`, `cli::parse_pr_view`, `cli::USAGE`).
  - `client` — `client::call(method, path, body)`: one std-only
    loopback HTTP/1.1 call that starts the daemon if none answers.
  - `endpoint` — the `<state dir>/endpoint-p<PROTOCOL>` file (`port token`); keyed by wire protocol so a client never uses an incompatible daemon (#31).
  - `fields` — `PR_FIELDS` and `valid_name`, the GraphQL allowlist.
  - `proto` — `PrViewRequest` / `PrViewResponse`.
  - `version_banner()` — `ghd <version> (<os>)`.
- Only with the `daemon` feature (from `ghd-daemon`):
  - `run_daemon()` — run the loopback server until it exits.

## Why the only `cfg` is on a `pub use`

`#[cfg(feature = "daemon")]` decides whether a name exists, never what
it does. If behavior ever needed a feature branch, that behavior would
belong in a private crate, gated by its `dep:` line in `Cargo.toml`.
That keeps `ci.toml`'s two ship sets (`[]` and `["daemon"]`) the only
configurations that need testing.

## Who consumes it

- `crates/ghd-cli` — with `features = ["daemon"]`, because the `ghd`
  binary dispatches `ghd daemon` to `run_daemon`.
- `crates/ghd-py` — default features: the extension only calls a
  running daemon through `cli::pr_view_request`.

Neither depends on `ghd-core` or `ghd-daemon` directly.

## Testing

`Cargo.toml` sets `[lib] test = false`; there is nothing here to
unit-test. The surface is checked by `../tests/api/main.rs` under both
ship sets:

```bash
soldr cargo test -p ghd --locked
soldr cargo test -p ghd --locked --features daemon
```

## Changing this file

Add a re-export only after the symbol exists (and is tested) in its
private crate, then extend `tests/api/main.rs` to prove it is reachable.
