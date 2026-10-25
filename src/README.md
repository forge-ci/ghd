# `src/`

The Python source layout for the wheel. Single package:
`ghd`, declared in `pyproject.toml` as the
maturin `python-source` (consumed by the soldr PEP 517 backend, which
drives a pinned maturin under the hood — see `pyproject.toml`'s
`[build-system]`).

## Layout

```
src/ghd/
├── __init__.py        # package version + re-exports
├── _native.pyi        # typing stub for the PyO3 surface
├── bindings.py        # Python wrapper around the extension module
└── platforms/          # host-platform facade — see its own README
```

The `ghd-cli` native binary is NOT shipped under this package
directory. The soldr PEP 517 backend bundles it straight into the
wheel's `<name>-<ver>.data/scripts/` directory at build time
(`[tool.soldr.pep517] bundle-bins`) and pip drops it straight into the
venv's `Scripts/` (Windows) or `bin/` (POSIX) directory at install
time — no Python wrapper sits in front of it. See
`ghd/README.md` for the rationale (issue #2,
items 1 + 10).

## Why `src/`-layout instead of flat

Standard PEP 517 src-layout avoids the "accidentally importing
half-built package" failure mode where the working directory shadows
the installed package. Tools that read `pyproject.toml` (pytest, uv,
maturin) all support src-layout out of the box, so there's no friction
for it.

## What you can edit here

- `bindings.py` — the public Python API. Each function should be a
  near-1:1 reflection of an underlying `_native` call, with type
  annotations and a one-line docstring.
- `platforms/` — host checks only. See `platforms/README.md`; nowhere
  else in this package may call `sys.platform`/`os.name`.
- `__init__.py` — package version, public re-exports. Don't import
  `_native` directly here; route through `bindings.py`.
- `_native.pyi` — optional typing stub mirroring the PyO3 surface.
  Keep it in sync with `crates/ghd-py/src/lib.rs`.

## What you should NOT edit here

- `_native*.pyd` / `_native*.so` / `_native*.dylib` — built by the
  soldr PEP 517 backend, gitignored.
- Anything implementing domain logic — that belongs in `ghd-core`
  (reached through the `ghd` amalgam).
- A `sys.platform`/`os.name` check outside `platforms/` — see above.

## Build modes

| Mode               | Command                                            | What's materialized                |
|--------------------|-----------------------------------------------------|------------------------------------|
| Project sync (dev) | `uv sync`                                           | `_native*.so`/`.pyd` installed into `.venv`, plus `ghd-cli` bundled on `.venv`'s PATH |
| Release wheel      | `uv build --wheel`                                  | wheel under `dist/`                |
| Sdist              | `uv build --sdist`                                  | tarball under `dist/`              |

Both `uv build` modes go through the same soldr PEP 517 backend
(`pyproject.toml`'s `[build-system]`) — never a direct `maturin`
invocation. See `ci/gates/test.py` and `ci/gates/backend_smoke.py` for
the canonical gate-level commands.
