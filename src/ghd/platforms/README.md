# `ghd.platforms`

The host-platform facade for the Python side of this repo. Mirrors the
Rust `ghd-platform` crate (`crates/private/ghd-platform`):
one small module owns every host check, and nothing else in the tree
calls `sys.platform`, `os.name`, or `platform.system()` directly.

## Why this exists

[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) round 1
declares exactly two allowed locations for host-branching code in this
workspace (`ci.toml`'s `[allow] platform-code`):
`crates/private/ghd-platform/src/platforms/**` on the Rust side,
and this directory on the Python side. Before this round, host checks
were scattered inline — `tests/test_cli.py` computed its own
`BINARY_NAME` with `os.name == "nt"`, `ci/gates/action_surface.py` and
`ci/gates/backend_smoke.py` each had their own `sys.platform` check.
Centralizing them here means a future lint (the Python analogue of the
Rust boundary Dylint) can enforce the rule mechanically: grep the tree
for `sys.platform`/`os.name`/`platform.system()` outside this one
directory and `crates/private/ghd-platform/src/platforms/**`.

## Public surface

```python
is_windows() -> bool
is_linux() -> bool
is_macos() -> bool
os_name() -> str            # "windows" | "linux" | "macos"
binary_suffix() -> str      # ".exe" on Windows, "" elsewhere
cli_binary_name() -> str    # f"ghd-cli{binary_suffix()}"
```

`os_name()` raises `RuntimeError` on an unrecognized host rather than
guessing — the same "no silent fallback" rule the Rust facade's
`cfg_select!` enforces at compile time (there, an unsupported OS fails
to *compile*; here, it fails loudly at import-adjacent call time
instead, since Python has no compile-time host selection).

## Zero runtime dependencies, on purpose

This module imports nothing but `sys` from the standard library. That
matters because it's imported from places that must NOT trigger a full
project build: `ci/gates/action_surface.py` and
`ci/gates/backend_smoke.py` (both `sys.path.insert` this `src/`
directory and import straight from it) need a host check *before* the
`_native` extension necessarily exists. Do not add an import here that
would make importing this module require the built extension, `uv
sync`, or anything else beyond a bare interpreter.

## Who uses this today

- `src/ghd/bindings.py` — none yet; reserved for
  when the Python surface needs a host-specific answer of its own.
- `tests/test_cli.py` — `cli_binary_name()`, to assert the installed
  binary's filename matches this host's convention.
- `ci/gates/action_surface.py` — `cli_binary_name()`.
- `ci/gates/backend_smoke.py` — `is_linux()`, to skip on platforms
  where soldr's compile daemon can't spawn (zackees/soldr#1300).

## Adding a check

Add a function here, not an inline `sys.platform`/`os.name` check at
the call site. If the new check needs a real dependency (not just
`sys`), reconsider whether it belongs in this always-cheap-to-import
module at all — see "Zero runtime dependencies" above.
