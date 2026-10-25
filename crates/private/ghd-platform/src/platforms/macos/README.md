# `crates/private/ghd-platform/src/platforms/macos/`

The macOS implementation tree. Compiled only when `../../lib.rs`'s
`cfg_select!` takes the `target_os = "macos"` arm (as
`mod platform_macos`, aliased `crate::platform_imp`). It is the only
place in the workspace allowed to assume a macOS host.

## Files

| File | Provides |
|---|---|
| `mod.rs` | `pub(crate) mod fs; pub(crate) mod host; pub(crate) mod process;` |
| `host.rs` | `os_name()` → `"macos"`; `exe_suffix()` → `""` |
| `fs.rs` | `state_base()`, `create_private_dir()`, `write_private()` |
| `process.rs` | `spawn_detached()` |

## Behavior

- **State base.** `$HOME/Library/Application Support`; an error if
  `HOME` is unset. The facade appends `ghd`, so the daemon's
  `endpoint`, `start.lock` and `daemon.log` live in
  `~/Library/Application Support/ghd/`. That is the per-user location
  macOS apps use for their own persistent data; XDG variables are not
  consulted on this host.
- **Private directory.** `create_dir_all`, then mode `0700`.
- **Private file.** Writes a `.tmp` sibling opened with mode `0600`,
  `sync_all`, then `rename` over the target. The rename makes the
  replacement atomic, so a client never reads a half-written endpoint
  file; the mode keeps the bearer token away from other users.
- **Detached spawn.** `Command::process_group(0)`: the daemon gets its
  own process group, so a Ctrl-C or closed terminal in the session that
  started it does not reach it.

## Why literals, not detection

`os_name` returns a fixed `"macos"`. The fact that this file compiled
at all already proves `target_os = "macos"`; a runtime check would be a
second, weaker source of truth. `exe_suffix` is empty because that is
macOS's real convention.

## Relationship to the Linux tree

`create_private_dir`, `write_private` and `spawn_detached` are the same
code as in `../linux/`. They are duplicated rather than shared via a
`unix` arm so that `lib.rs` remains the only selector, and so the two
hosts can diverge independently. Only `state_base()` differs today.

## Rules

- Everything is `pub(crate)`; no other crate names this module.
- Unix-only std APIs (`std::os::unix::*`) are allowed here and nowhere
  outside `platforms/`.
- Any new function must also be added to `../linux/` and `../windows/`
  and exposed through `../../platform/`.

## Testing

Exercised through the facade by `ghd-platform`'s unit tests on a macOS
host; from Linux, `soldr cargo check --target <apple triple>` proves it
compiles.

```bash
soldr cargo test -p ghd-platform --locked
```
