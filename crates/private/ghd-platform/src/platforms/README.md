# `crates/private/ghd-platform/src/platforms/`

Native implementations, one subdirectory per supported host. This is
the only tree in the workspace where code may assume a particular OS.

```
platforms/
├── linux/     # mod.rs, fs.rs, host.rs, process.rs
├── macos/     # mod.rs, fs.rs, host.rs, process.rs
└── windows/   # mod.rs, fs.rs, host.rs, process.rs
```

## How a tree gets compiled

None of these directories is a normal `mod` of the crate. `../lib.rs`
names each with `#[path = "platforms/<os>/mod.rs"]` inside one arm of
its `cfg_select!`, and aliases the chosen one as `crate::platform_imp`.
On any given host exactly one tree compiles; the other two are not even
parsed into the build. There is no fallback tree.

## Why three full trees instead of shared `unix` code

Linux and macOS currently have identical `create_private_dir`,
`write_private` and `spawn_detached`. They are duplicated, not shared
through a `unix` arm, on purpose: a shared arm would be a second
selector, and the two hosts genuinely differ elsewhere (`state_base()`
is `$XDG_STATE_HOME` or `~/.local/state` on Linux, `~/Library/Application
Support` on macOS). Duplication keeps each host readable in one place
and lets either diverge without touching the other.

## What each tree must provide

Each `mod.rs` declares `pub(crate) mod fs; pub(crate) mod host;
pub(crate) mod process;`, and each leaf provides the `pub(crate)`
functions the facade in `../platform/` forwards to:

| Leaf | Functions |
|---|---|
| `host.rs` | `os_name()`, `exe_suffix()` |
| `fs.rs` | `state_base()`, `create_private_dir(dir)`, `write_private(path, bytes)` |
| `process.rs` | `spawn_detached(command)` |

Everything is `pub(crate)`: no other crate may name a concrete
`platforms::*` module. Callers go through the facade.

## Summary of host differences

| | Linux | macOS | Windows |
|---|---|---|---|
| state base | `$XDG_STATE_HOME` or `~/.local/state` | `~/Library/Application Support` | `%LOCALAPPDATA%` |
| privacy | dir `0700`, file `0600` | dir `0700`, file `0600` | profile ACL inherited |
| detach | `process_group(0)` | `process_group(0)` | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP` |
| exe suffix | `""` | `""` | `".exe"` |

## Adding a host or capability

A new capability needs a function in all three trees (plus the facade).
A new host needs a new tree and a new `cfg_select!` arm in `../lib.rs`
and must be added to `ci.toml`'s `[platforms]`.

## Testing

`soldr cargo test -p ghd-platform --locked` on each host runs the facade
tests against that host's tree.
