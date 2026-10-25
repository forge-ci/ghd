# `ghd`

The public API amalgam. This is the **only** crate in the workspace
published to crates.io, and the **only** crate `ghd-cli` and `ghd-py`
depend on for domain behavior. It carries no logic of its own — every
symbol is a `pub use` re-export from a private crate under
`crates/private/`.

## Why an amalgam instead of publishing `ghd-core` directly

Two different questions would otherwise share one crate: "what does the
code do" and "what does a consumer of this workspace see." Splitting
them means a capability (`ghd-daemon`, with its kernal-api async/HTTP
stack) can be added, compiled once, unit-tested once, and toggled on/off
for consumers without touching `ghd-core`'s own API or compile graph. A
library caller that only talks to a running daemon never links the
daemon side.

## Features are `dep:` wiring, nothing else

```toml
[features]
daemon = ["dep:ghd-daemon"]
```

That is the **entire** feature surface. `ci.toml`'s `[rust] ship = [[],
["daemon"]]` names the only two feature sets this crate is ever compiled
or tested with. The one `#[cfg(feature = "daemon")]` in `lib.rs` gates
whether `run_daemon` is re-exported at all — never a behavioral branch.
If you find yourself writing `#[cfg(feature = "...")]` around actual
logic anywhere in this crate, that logic belongs in a private crate
instead, gated by the presence or absence of its `dep:` line.

## Public surface

```rust
pub use ghd_core::{cli, client, endpoint, fields, proto, version_banner}; // always
#[cfg(feature = "daemon")]
pub use ghd_daemon::run as run_daemon;                                   // only with `daemon`
```

- `cli` — `cli::run(&args)` (every subcommand except `daemon`) and
  `cli::pr_view_request` (used by `ghd-py`).
- `client` — one loopback HTTP call, auto-starting the daemon.
- `endpoint` — the `port token` file a running daemon writes.
- `fields` — `PR_FIELDS`, the `gh pr view --json` names ghd answers.
- `proto` — `PrViewRequest` / `PrViewResponse` (prost messages).
- `run_daemon` — blocks running the loopback server (`ghd daemon`).

`[lib] test = false`: no unit-test harness of its own; each re-export is
unit-tested once in its owning private crate.

## Testing

`ghd:test:api` in `ci.toml`'s `[rust.tests].binaries` —
`tests/api/main.rs`, this crate's one integration-test target. It is
run twice in a full validation pass: once with no features (the `[]`
ship set) and once with `--features daemon` (the `["daemon"]` ship
set), so both entries in `ci.toml`'s `[rust] ship` list are exercised,
not just compiled.

```bash
soldr cargo test -p ghd --locked                      # [] ship set
soldr cargo test -p ghd --locked --features daemon    # ["daemon"] ship set
```

## Adding a re-export

1. Implement the capability in a new or existing crate under
   `crates/private/` (see that directory's README for the rules).
2. If it's optional, add a Cargo feature here that's nothing but
   `dep:<crate>` — no other wiring.
3. Add the `pub use` (gated by `#[cfg(feature = ...)]` if optional).
4. Extend `tests/api/main.rs` to exercise it.
5. If it changes the ship sets, update `ci.toml`'s `[rust] ship` and
   this README.
