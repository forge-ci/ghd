# `crates/private/ghd-platform/src/platform/`

The host-neutral facade leaves. `../platform.rs` declares these three
modules and `../lib.rs` re-exports them as `ghd_platform::{fs, host,
process}`. Callers in other crates reach them as `crate::platform::fs`
etc.

## Files

| File | Functions | Delegates to |
|---|---|---|
| `host.rs` | `os_name() -> &'static str`, `exe_suffix() -> &'static str` | `platform_imp::host` |
| `fs.rs` | `state_dir()`, `create_private_dir(dir)`, `write_private(path, bytes)` | `platform_imp::fs` |
| `process.rs` | `spawn_detached(&mut Command) -> io::Result<Child>` | `platform_imp::process` |

## The rule for this directory

**No host `cfg` here.** No `#[cfg(target_os = ...)]`, `#[cfg(unix)]`,
`#[cfg(windows)]` or `cfg!(...)`. Each function forwards to
`crate::platform_imp::<leaf>`, the implementation `lib.rs` chose with
`cfg_select!`. A facade function may add host-neutral glue — `fs::
state_dir()` appends `ghd` to the host's `state_base()` — but never a
per-host branch.

Why: if a facade file branches on the host, the boundary has two
selection sites and it is no longer obvious where behavior differs.
Keeping all differences in `../platforms/<os>/` makes them auditable in
one place.

## Contracts each implementation must meet

- `host::os_name()` returns exactly `"windows"`, `"linux"` or `"macos"`.
- `host::exe_suffix()` returns `".exe"` on Windows, `""` elsewhere.
- `fs::state_dir()` is a per-user directory ending in `ghd`; it holds
  the daemon's `endpoint` file, `start.lock` and `daemon.log`.
- `fs::create_private_dir()` creates the directory (and parents)
  readable only by the current user.
- `fs::write_private()` atomically replaces the file (write a `.tmp`
  sibling, sync, rename), readable only by the current user. The
  endpoint file carries a bearer token, hence the privacy requirement.
- `process::spawn_detached()` starts the command so it outlives the
  caller and is not signalled with it (Ctrl-C, closed console).

## Adding a capability

1. Add the function here, forwarding to `crate::platform_imp::<leaf>`.
2. Implement it in all three of `../platforms/{linux,macos,windows}/`
   — a missing arm is a compile error on that host, by design.
3. Test it in `../lib.rs`'s `mod tests` through the facade.
4. Call it from other crates only as `crate::platform::<leaf>::...`.

## Testing

Covered by `ghd-platform:lib`:

```bash
soldr cargo test -p ghd-platform --locked
```
