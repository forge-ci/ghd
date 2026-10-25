# `crates/private/`

Every crate under this directory is `publish = false`. None of them
declare `[features]` or optional dependencies. Each is compiled and
unit-tested exactly once, and each is reachable from the outside world
only through the public [`ghd`](../ghd) amalgam's `pub use` re-exports.

## Why a directory, not just a naming convention

`ci.toml`'s `[rust] private = "crates/private/*"` glob is a structural
check, not a naming one: ci-lint can walk this one directory and
assert every member sets `publish = false` and carries no `[features]`
table, without having to guess which top-level `crates/*` entries are
"really" private by name alone. A crate that needs to stop being
private moves out of this directory (and becomes its own published
crate, or — far more commonly — folds into `ghd`'s feature table
instead of graduating on its own).

## Members

| Crate | What it owns |
|---|---|
| `ghd-core` | The caller's side of the daemon: `proto` (protobuf messages), `endpoint` (port/token file), `client` (std-only loopback HTTP/1.1, auto-start), `cli` (`ghd pr view`, `ghd stats`), `fields` (the answerable `--json` names / GraphQL allowlist). No runtime, no TLS. |
| `ghd-daemon` | The long-lived side: `server` (loopback routes over a per-PR cache: `fresh` → `revalidated` → `fetched`) and `upstream` (REST ETag sentinels, batched alias-merged GraphQL). Built only on kernal-api. |
| `ghd-platform` | The host-platform facade: one `cfg_select!` selector, a cfg-free surface (`host`, `fs`, `process`), native code confined to `src/platforms/**`. |

Dependency direction inside this directory: `ghd-daemon → ghd-core →
ghd-platform`. Never the reverse.

## Rules every crate here follows

- `publish = false` in `[package]`.
- No `[features]` table, no optional dependencies. Feature gating
  lives one layer up, in the public `ghd` amalgam's `dep:` wiring —
  never inside a private crate's own logic.
- `[lib] doctest = false` — these crates' API documentation job belongs
  to the amalgam.
- At least one real `#[test]` under `#[cfg(test)] mod tests` (a crate
  with a lib harness and zero tests is a precheck finding — an empty
  harness proves nothing).
- No `#[cfg(target_os = ...)]` / `#[cfg(windows)]` / `#[cfg(unix)]` /
  `cfg!(...)` anywhere except inside `ghd-platform`'s own
  `src/platforms/**` (and the one selector in its `lib.rs`). Everything
  else reaches the OS via `ghd-platform`'s facade.
- Async, HTTP and JSON only through kernal-api; no crate here names
  tokio, reqwest or hyper.

## What does NOT belong here

- Anything with its own `[features]` table — that's the public `ghd`
  crate's job.
- Anything that needs to be independently version-pinned by a
  downstream consumer — publish it as its own top-level `crates/<name>`
  instead.
- Binaries. `ghd-cli` (the crate that builds the one `ghd` binary this
  workspace ships) lives at `crates/ghd-cli`, one level up, because it
  is itself part of the public surface (bundled into the wheel).

## Adding a member

See `crates/README.md`'s "Adding a new crate" section — the steps are
the same, plus: register the new crate's test binary in `ci.toml`'s
`[rust.tests].binaries`, and wire it into `ghd`'s `[features]` table if
it should be reachable from outside the workspace at all.
