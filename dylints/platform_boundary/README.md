# `platform_boundary`

This repository's first Dylint library. A pre-expansion, `Deny` lint that
enforces the `platforms/` facade boundary described in the template's
`CLAUDE.md` rule 8 and `ci.toml`'s `[allow] platform-selector` /
`platform-code`:

- **`crates/private/ghd-platform/src/lib.rs`** is the ONLY source
  file in the workspace allowed to select the host platform: `#[cfg]`,
  `#[cfg_attr]`, `cfg!(...)`, and `cfg_select!` naming any of the ten host
  selectors (`windows`, `unix`, `target_os`, `target_family`,
  `target_arch`, `target_abi`, `target_env`, `target_vendor`,
  `target_endian`, `target_pointer_width`) are denied everywhere else.
- **`crates/private/ghd-platform/src/platforms/**`** may
  additionally reference native platform paths (`std::os::windows`,
  `std::os::unix`, `libc::`, `windows_sys::`) — denied everywhere else,
  including `lib.rs`... no, ALSO allowed in `lib.rs` itself (the selector
  file is exempt from both the cfg check and the native-path check; only
  the `platforms/**` trees get the native-path exemption without the cfg
  exemption, since — by this template's design — they carry no `cfg` of
  their own at all; see below).
- **The concrete module aliases** (`platform_imp`, `platform_windows`,
  `platform_linux`, `platform_macos`) may be referenced anywhere inside
  the `ghd-platform` crate (its cfg-free `platform.rs` facade
  re-exports them) but nowhere else — every other crate reaches only
  `crate::platform::host`.

Inspection happens **pre-expansion**, so `cfg`'d-away code cannot hide
from it, and it runs from a single Linux job against the checked-out
source, not per-target — see "why one Linux Dylint job is sufficient for
the selector part of the boundary" in `src/lib.rs`'s lint doc comment.

## Why this crate's boundary shape differs from soldr's

Soldr's `dylints/ban_platform_cfg_outside_boundary` (the pattern this
lint is adapted from — see Attribution below) allows THREE concrete
per-OS trees (`platform_win`, `platform_linux`, `platform_macos`) to carry
their OWN `#[cfg(target_os = …)]`, because soldr's platform crate selects
with `cfg_select!` *and* keeps a `cfg`-gated fallback shape in those
trees. This template's `platforms/**` trees carry **no `cfg` of their
own** — `lib.rs`'s `cfg_select!` selects the module via `#[path = …]`
conditional module declarations, so the trees themselves never need to
ask "which OS am I." That is why this lint has ONE selector file, not
one selector file plus three cfg-carrying trees, and why the cfg check
excludes only `lib.rs` while the native-path check additionally exempts
`platforms/**`.

## RED → GREEN

Add a temporary host-boundary violation anywhere in
`crates/private/ghd-core/src/lib.rs` (outside the boundary) on a
throwaway commit: the `dylint` job fails with `PLATFORM_BOUNDARY`,
naming the file and the exact clause. Revert the commit: the job goes
green again.

Captured on zackees/ci.yml#6 round 2B (PR #17), once the lane's driver
acquisition was fixed to request a catalogued nightly
(`nightly-2026-05-28` — see this crate's `rust-toolchain.toml`):

- **RED** — run
  [36493899806](https://github.com/forge-ci/ghd/actions/runs/36493899806),
  job `109168891535`, 1m27s. Fixture: a direct `platform_imp` reference
  in `ghd-core` (a concrete-tree reference, not a `cfg` pattern, so
  precheck's `LAYOUT-001` regex scan does not fire — only the compiled
  lint catches it). Exact diagnostic:
  `error: host-platform selection outside the ghd-platform
  boundary: direct concrete-tree reference `platform_imp`; the only
  allowed selection site is crates/private/ghd-platform/src/lib.rs,
  and native platform paths are additionally allowed under
  crates/private/ghd-platform/src/platforms/**` — reported on
  `crates/private/ghd-core/src/lib.rs:16` (the file's first item;
  this lint scans the whole file once and reports on that item's span,
  not necessarily the exact line of the violating token).
- **GREEN** — run
  [36494259356](https://github.com/forge-ci/ghd/actions/runs/36494259356),
  job `109170048076`, 3m28s, after reverting the fixture.

An earlier attempt in the same PR (a `#[cfg(windows)]` marker) also
tripped precheck's `LAYOUT-001` in the same commit, which blocks the
`dylint` job entirely (precheck gates it) — useful precheck-rule
RED/GREEN evidence (see `.github/workflows/README.md`) but not a
dylint-lane-isolated signal, which is why the captured pair above uses
a concrete-tree reference instead.

## Tests

- **`ui/`** — `dylint_testing::ui_test` fixtures, loaded into a real
  Dylint driver against this crate's own pinned nightly:
  - `allowed_boundary.rs` (green) — stands in for what the selector file
    and `platforms/**` are allowed to do (excluded from the lint's scope
    by filename, the same way soldr's own UI fixture is).
  - `disallowed_cfg_macro_in_body.rs`, `disallowed_concrete_reference.rs`,
    `disallowed_native_path.rs`, `disallowed_cfg_select.rs` (red) — each
    pairs with a `.stderr` fixture recording the exact expected
    diagnostic.
- **Plain `#[test]`s in `src/lib.rs`** — pure unit tests for the
  comment/string-aware scanner functions (`platform_cfg_invocations`,
  `contains_cfg_select`, `concrete_tree_references`,
  `native_platform_references`, `in_scope`, `relative_path`), independent
  of the rustc driver. Run these with `soldr cargo test --lib` from this
  directory for a fast local iteration loop; the full `ui` test needs the
  nightly driver Soldr's `dylint: true` mode prepares in CI.

## Local dev note

This crate needs `rustc-dev`/`llvm-tools-preview` for its pinned nightly
and, the first time that nightly has no catalogued prebuilt Dylint
driver, a local source build of `dylint_driver` (soldr's "binary-or-exit"
policy — set `SOLDR_ALLOW_DYLINT_DRIVER_BUILD=1` to permit the one-time
local fallback build; CI's `setup-soldr` `dylint: true` mode resolves a
catalogued driver instead and never needs this). Run
`soldr cargo test` from inside `dylints/platform_boundary/`.

## Attribution

Scan logic and pre-expansion pattern adapted from soldr's
`dylints/ban_platform_cfg_outside_boundary`
(zackees/soldr issues #2493, #2498), itself following kernal-api's
`docs/platform-boundary.md`. See `src/lib.rs`'s lint doc comment for the
detailed attribution and what changed for this workspace's shape.
