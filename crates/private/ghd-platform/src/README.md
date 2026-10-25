# `crates/private/ghd-platform/src/`

The host-platform facade's source. This directory is where the
workspace's only host selection happens, and where all native code is
confined.

## Layout

```
src/
├── lib.rs           # the one cfg_select! + `pub use platform::{fs, host, process}`
├── platform.rs      # facade index (no host cfg)
├── platform/        # facade leaves: fs.rs, host.rs, process.rs (no host cfg)
└── platforms/       # native implementations, one tree per host
    ├── linux/
    ├── macos/
    └── windows/
```

## `lib.rs`

`cfg_select!` picks exactly one of `platforms/windows/mod.rs`,
`platforms/linux/mod.rs` or `platforms/macos/mod.rs` and aliases it as
`crate::platform_imp`. There is no fallback arm and no `unix` arm: an
unsupported host fails to compile here, loudly, instead of guessing.

It re-exports the facade (`fs`, `host`, `process`) and holds the crate's
unit tests, which go through the facade, never `cfg`:

- `host::os_name()` is one of `windows`/`linux`/`macos`.
- `host::exe_suffix()` equals `std::env::consts::EXE_SUFFIX`.
- the two agree on whether the host is Windows.

## The facade surface

| Function | Used by |
|---|---|
| `host::os_name()` | `ghd-core::version_banner` |
| `host::exe_suffix()` | `ghd-core::client` (finding `ghd` on PATH) |
| `fs::state_dir()` | `ghd-core::endpoint` (endpoint file, `start.lock`, `daemon.log`) |
| `fs::create_private_dir()` / `fs::write_private()` | endpoint file and state dir |
| `process::spawn_detached()` | starting `ghd daemon` |

## Why this shape

Every other crate aliases this one as `crate::platform` and stays free
of `#[cfg(target_os = ...)]`, `#[cfg(windows)]`, `#[cfg(unix)]` and
`cfg!(...)`. Reviewers then only need to audit one selector and the
`platforms/**` tree for host-specific behavior. The pattern follows
soldr's `soldr-platform` and kernal-api's `docs/platform-boundary.md`.

## Dependency direction

Leaf crate: no workspace or third-party dependencies, std only.
`ghd-core` depends on it; nothing here depends on another ghd crate.

## Testing

```bash
soldr cargo test -p ghd-platform --locked
```

`ghd-platform:lib` in `ci.toml`. Each CI host runs the tests for its
own arm; other arms are covered by cross-target `soldr cargo check`.
