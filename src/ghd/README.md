# `ghd`

The actual Python package. Imported as `import ghd` after installing
the wheel. It gives Python callers (for example `pr_merge_watch`)
cached `gh pr view --json` reads through the ghd daemon, in-process.

## Public modules

| Module        | Purpose                                                                 |
|---------------|-------------------------------------------------------------------------|
| `bindings`    | Python wrappers around the PyO3 extension. Public API surface.          |
| `platforms`   | Host-platform facade — see `platforms/README.md`. The only place in this package allowed to call `sys.platform`/`os.name`. |
| `__init__`    | Package version (`__version__`).                                        |

## Internal modules

| Module        | Purpose                                                                 |
|---------------|-------------------------------------------------------------------------|
| `_native`     | Built by the soldr PEP 517 backend from `crates/ghd-py`. Never import directly from outside this package — go through `bindings`. **No pure-Python fallback** — see below. |
| `_native.pyi` | Typing stub for IDEs: `version_banner()` and `pr_view(owner, repo, number, fields) -> str`. |

## The API (`bindings.py`)

```python
from ghd.bindings import pr_view, version_banner

version_banner()                                   # "ghd 0.1.0 (linux)"
pr_view("owner/repo", 123, ["state", "title"])     # {"state": ..., "title": ...}
```

- `version_banner()` returns `ghd <version> (<os>)` from the Rust core.
- `pr_view(repo, number, fields)` splits `repo` into owner/name, calls
  `_native.pr_view`, and `json.loads` the document gh would print.
  It starts the daemon (the bundled `ghd` binary) on first use. Fields
  must be names from `ghd-core`'s `fields::PR_FIELDS`; anything else is
  rejected by the daemon with HTTP 400, surfaced as `OSError`.

Why wrap instead of re-export? So the Python surface stays stable even
if the underlying PyO3 signature changes (the native `pr_view` takes
`owner, repo` separately and returns a string; the wrapper takes
`"owner/repo"` and returns a `dict`). The wrapper is the unit of API
compatibility.

`_native` has **no pure-Python fallback** — `bindings.py` imports it
unconditionally, no `try`/`except ImportError`. If the extension isn't
built, importing this module fails loudly instead of silently degrading
(`ci.toml`'s packaging check, `PKG-005`).

## CLI delivery (no Python shim)

The `ghd` binary on PATH after install is the cargo-built executable
itself (crate `ghd-cli`), **not** a Python launcher. The soldr PEP 517
backend bundles it straight into the wheel's `<name>-<ver>.data/scripts/`
directory (`[tool.soldr.pep517] bundle-bins` in `pyproject.toml`).
Files in `.data/scripts/` land in the venv's `Scripts/` (Windows) or
`bin/` (POSIX) directory verbatim — `.exe` files are NOT wrapped.

Why no Python shim? On Windows, `[project.scripts]` generates a
console-script `.exe` whose `os.execv` is emulated as `CreateProcess`
+ parent exit, so the shell prompt races ahead of the native binary's
output. See [fbuild#747](https://github.com/FastLED/fbuild/pull/747)
and [issue #2](https://github.com/forge-ci/ghd/issues/2) items (1) + (10).

The binary is also how the extension gets a daemon: when no daemon
answers, the Rust client spawns `ghd daemon` from `$GHD_EXE` or `ghd`
on PATH.

## Why both an extension AND a binary?

- **In-process API.** A Python program reads PR state without spawning
  a subprocess per call. → Use `bindings.py`.
- **Command surface.** A user, shell script or CI step runs
  `ghd pr view 123 --json state`. → Use the `ghd` binary.

Both go through `ghd-core`, so they honor the same behavior and share
the same daemon and cache.

`bindings.gh_read(args)` returns a frozen `ReadResult` carrying output bytes,
exit code, cache source and a conservative list-limit marker. It calls the
same Rust reader as the CLI, and rejects write/interactive/unbound shapes.
Nested PR fields are also available through the existing `pr_view` binding.
