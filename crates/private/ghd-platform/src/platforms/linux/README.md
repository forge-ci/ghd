# `crates/private/ghd-platform/src/platforms/linux/`

The Linux implementation tree. Compiled only when `../../lib.rs`'s
`cfg_select!` takes the `target_os = "linux"` arm (as
`mod platform_linux`, aliased `crate::platform_imp`). It is the only
place in the workspace allowed to assume a Linux host.

## Files

| File | Provides |
|---|---|
| `mod.rs` | `pub(crate) mod fs; pub(crate) mod host; pub(crate) mod process;` |
| `host.rs` | `os_name()` → `"linux"`; `exe_suffix()` → `""` |
| `fs.rs` | `state_base()`, `create_private_dir()`, `write_private()` |
| `process.rs` | `spawn_detached()` |

## Behavior

- **State base.** `$XDG_STATE_HOME` if set and non-empty, else
  `$HOME/.local/state`; an error if `HOME` is unset. The facade appends
  `ghd`, so the daemon's `endpoint`, `start.lock` and `daemon.log` live
  in e.g. `~/.local/state/ghd/`. XDG's *state* directory fits: this is
  runtime state that should survive restarts but is not configuration
  or cache.
- **Private directory.** `create_dir_all`, then mode `0700`.
- **Private file.** Writes a `.tmp` sibling opened with mode `0600`,
  `sync_all`, then `rename` over the target. The rename makes the
  replacement atomic, so a client never reads a half-written endpoint
  file; the mode keeps the bearer token away from other users.
- **Detached spawn.** `Command::process_group(0)`: the daemon gets its
  own process group, so a Ctrl-C or session teardown in the agent's
  terminal that started it does not reach it.

## Why literals, not detection

`os_name` returns a fixed `"linux"`. The fact that this file compiled
at all already proves `target_os = "linux"`; detecting at runtime would
add a second, weaker source of truth. `exe_suffix` is empty because
that is Linux's real convention, not a placeholder.

## Relationship to the macOS tree

`create_private_dir`, `write_private` and `spawn_detached` are the
same code as in `../macos/`. They are duplicated rather than shared via
a `unix` arm so that `lib.rs` stays the only selector; only
`state_base()` differs today.

## Rules

- Everything is `pub(crate)`; no other crate names this module.
- Unix-only std APIs (`std::os::unix::*`) are allowed here and nowhere
  outside `platforms/`.
- Any new function must also be added to `../macos/` and `../windows/`
  and exposed through `../../platform/`.

## Testing

Exercised through the facade by `ghd-platform`'s unit tests on a Linux
host:

```bash
soldr cargo test -p ghd-platform --locked
```
