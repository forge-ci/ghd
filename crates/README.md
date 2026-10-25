# `crates/`

The Rust workspace for `ghd`. Shaped by [zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6)
(round 1) around one rule: **one published amalgam, several unpublished
crates that are each compiled and tested exactly once.**

## Layout

```
crates/
├── ghd/                  # PUBLIC: the amalgam. pub use re-exports only.
├── ghd-cli/              # PUBLIC-ish: the `ghd` binary. Bundled into the
│                         # wheel by soldr (bundle-bins).
├── ghd-py/               # PUBLIC-ish: the PyO3 `_native` extension.
└── private/              # publish = false. No [features]. No optional deps.
    ├── ghd-core/         # client side: proto, endpoint, client, cli, fields
    ├── ghd-daemon/       # daemon side: server (routes + cache), upstream (GitHub)
    └── ghd-platform/     # the host-platform facade: host, fs, process
```

## Dependency direction (don't break this)

```
ghd-cli ──► ghd[daemon] ──┬──► ghd-core ──► ghd-platform
ghd-py  ──► ghd       ────┘
                          └──► ghd-daemon ──► ghd-core   (only with `daemon`)
```

`ghd` is the only crate `ghd-cli` and `ghd-py` depend on — never
`ghd-core`/`ghd-daemon` directly. `ghd` itself carries no logic: it is
`pub use` re-exports gated by one Cargo feature
(`daemon = ["dep:ghd-daemon"]`), so the private crates stay compiled and
unit-tested exactly once regardless of how many public surfaces consume
them. `ghd-cli` enables `daemon` because `ghd daemon` runs the server;
`ghd-py` does not, because Python callers only talk to a running daemon
(the bundled `ghd` binary is what gets spawned).

The split also keeps the hot path cheap: `ghd-core` has no async runtime
and no TLS (std-only HTTP/1.1 to loopback), while everything that needs
kernal-api's async engine, HTTP client/server and JSON lives in
`ghd-daemon`.

## Why a public/private split

Without it, every new capability either bloats one crate's API or grows
a second public crate — no controlled way to ship an optional capability
without shipping its compile cost to every consumer. `crates/private/*`
fixes that: each private crate is `publish = false`, declares no
`[features]` of its own, and is wired into the public surface with plain
`dep:` feature gating on the `ghd` amalgam. Adding a capability means
adding a private crate and one line in `ghd`'s `[features]` table —
never a `#[cfg(feature = ...)]` branch inside shared logic.

## The platform facade

`ghd-core` (and, transitively, everything else) reaches the OS through
exactly one door: `crate::platform::*`, aliased from `ghd-platform`. See
`crates/private/ghd-platform/README.md` for the full pattern — it is the
same shape soldr, zccache, and kernal-api use, confined by `ci.toml`'s
`[allow] platform-selector` / `platform-code` and (eventually) a Dylint
boundary check.

## Adding a new crate

1. `soldr cargo new --lib crates/private/<name>` (or the public
   `crates/<name>` if it genuinely needs its own published artifact —
   rare; prefer adding to `ghd`'s feature table instead).
2. Add `<name>` to `members` in the root `Cargo.toml`.
3. Use `version.workspace = true`, `edition.workspace = true`, and the
   other inherited package fields.
4. Private crates: `publish = false`, no `[features]`, no optional
   deps, `[lib] doctest = false`, and at least one real `#[test]`.
5. Add a README.md (this directory's `readme_guard` requires it).
6. Register the crate's test binary in `ci.toml`'s
   `[rust.tests].binaries` — an undeclared test binary is a precheck
   finding (`ci.yml#6` §2 group 7).

## Workspace conventions

- `Cargo.toml` at the repo root owns `version`, `edition`,
  `rust-version`, `license`, `repository`, `homepage`, and
  `[workspace.metadata.soldr] targets` (must match `ci.toml`'s
  `[platforms]`). Member crates inherit the package fields with
  `.workspace = true`.
- Shared deps go in `[workspace.dependencies]` (`kernal-api`, `prost`,
  `pyo3`, `anyhow`); member crates pin with `{ workspace = true }`.
- Toolchain is pinned by `rust-toolchain.toml` at the repo root; do
  not override per-crate.
- Every Rust command goes through `soldr` (`soldr cargo ...`) — never
  bare `cargo`. See `ci/gates/*.py` for the canonical invocations.
