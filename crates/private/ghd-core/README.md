# `ghd-core`

Pure Rust domain logic. No Python, no CLI surface, no I/O policy. This
is the crate `ghd` (the public amalgam) re-exports from and every
optional private crate (`ghd-daemon`) depends on — keep it clean.

`publish = false`: see `crates/private/README.md` for the rules every
crate under `crates/private/` follows.

## What belongs here

- Types that model the problem domain.
- Algorithms over those types.
- Pure functions and value-semantics structs.
- `Result<T, anyhow::Error>` (or a domain-specific error enum) as the
  error contract — never `panic!` on recoverable conditions.

## What does NOT belong here

- `clap` or any other argv parser — that's `ghd-cli`'s job.
- `pyo3` decorators or `PyResult` — that's `ghd-py`'s job.
- `println!` / `eprintln!` for user output — domain code returns
  values; the binary decides how to render them (`run_cli` is the one
  deliberate exception: it exists specifically so the CLI and the
  facade tests share one "print the banner" code path).
- `[features]`, optional dependencies, or `#[cfg(feature = ...)]` —
  that's the public `ghd` amalgam's job.
- Process exits, signal handling, locale negotiation.
- Direct filesystem, network, or OS-facts I/O — that's
  `ghd-platform`'s job. This crate reaches the OS in exactly one
  way: `pub(crate) use ghd_platform as platform;`, then
  `platform::host::*`. No `#[cfg(target_os = ...)]`, `#[cfg(windows)]`,
  `#[cfg(unix)]`, or `cfg!(...)` appears anywhere in this crate.

## Why this crate is load-bearing

`ghd` re-exports it verbatim, and `ghd-cli`/`ghd-py`
(through `ghd`) translate its domain types into their respective
surfaces. If domain behavior lived anywhere else, the two surfaces
would drift — the CLI ships one bug fix, the bindings ship a different
one, and consumers see inconsistency. The whole point of the workspace
shape is to make divergence structurally hard.

## Public surface

Everything exported from `lib.rs` is a candidate for `ghd`'s
`pub use` re-export list. Be conservative:

- Mark items `pub` only when a consumer needs them.
- Prefer `pub use` re-exports over inline `pub mod` so the surface is
  greppable in one place.
- Match the `ghd-py` Python surface name-for-name where possible
  — `do_thing()` in `ghd-core` should be `do_thing()` in both the
  CLI and the bindings.

## Testing

Unit tests live next to their code under `#[cfg(test)] mod tests` —
`ghd-core:lib` in `ci.toml`'s `[rust.tests].binaries`. `[lib]
doctest = false`: this crate has no public-facing doctests of its own;
`ghd`'s doctests cover the re-exported API. Run with `soldr cargo
test --workspace --locked` (which `./ci.py test` / `./test` call).
