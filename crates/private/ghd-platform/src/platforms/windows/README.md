# `crates/private/ghd-platform/src/platforms/windows/`

The Windows implementation tree. Compiled only when `../../lib.rs`'s
`cfg_select!` takes the `target_os = "windows"` arm (as
`mod platform_windows`, aliased `crate::platform_imp`). It is the only
place in the workspace allowed to assume a Windows host.

## Files

| File | Provides |
|---|---|
| `mod.rs` | `pub(crate) mod fs; pub(crate) mod host; pub(crate) mod process;` |
| `host.rs` | `os_name()` → `"windows"`; `exe_suffix()` → `".exe"` |
| `fs.rs` | `state_base()`, `create_private_dir()`, `write_private()` |
| `process.rs` | `spawn_detached()` |

## Behavior

- **State base.** `%LOCALAPPDATA%`; an error if it is unset or empty.
  The facade appends `ghd`, so the daemon's `endpoint`, `start.lock`
  and `daemon.log` live in `%LOCALAPPDATA%\ghd\`. Local (not roaming)
  app data fits per-machine runtime state.
- **Private directory.** `create_dir_all` only. Files under the user
  profile inherit its owner-only ACL, so no extra DACL work is done.
- **Private file.** Writes a `.tmp` sibling with `File::create`,
  `sync_all`, then `rename` over the target, so the replacement is
  atomic. Privacy comes from the inherited profile ACL, not a mode.
- **Detached spawn.** `creation_flags(DETACHED_PROCESS |
  CREATE_NEW_PROCESS_GROUP)` (`0x8 | 0x200`): the daemon has no console
  and its own process group, so closing the console that started it,
  or a Ctrl-C there, does not take the daemon with it.

## Why literals, not detection

`os_name` returns a fixed `"windows"`; compiling this file already
proves the target. `exe_suffix` is `".exe"`, Windows's real convention.
`ghd-core`'s client uses it to find `ghd.exe` on PATH when the Python
binding needs to start the daemon.

## Differences from the Unix trees

Unlike `../linux/` and `../macos/`, nothing here sets Unix permission
bits (`std::os::unix` does not exist on this target), and detaching is
done with creation flags instead of `process_group(0)`. That divergence
is exactly why each host has its own tree.

## Rules

- Everything is `pub(crate)`; no other crate names this module.
- Windows-only std APIs (`std::os::windows::*`) are allowed here and
  nowhere outside `platforms/`.
- Any new function must also be added to `../linux/` and `../macos/`
  and exposed through `../../platform/`.

## Testing

Exercised through the facade by `ghd-platform`'s unit tests on a
Windows host; from Linux, `soldr cargo check --target <windows triple>`
proves it compiles.

```bash
soldr cargo test -p ghd-platform --locked
```
