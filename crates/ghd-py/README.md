# `ghd-py`

PyO3 extension crate that turns the `ghd` amalgam's client API into a
Python extension module. Built as a `cdylib` named `_native`, loaded
into Python as `ghd._native`. Depends on `ghd` **without** the `daemon`
feature — never on `ghd-core`/`ghd-daemon` directly (see
`crates/README.md`'s dependency-direction diagram). The daemon is the
bundled `ghd` binary; this module only talks to it.

## Why it exists

Python tools such as `pr_merge_watch` read PR state constantly. Going
through the extension means a read is one loopback HTTP round trip from
inside the Python process — no `gh`/`ghd` subprocess per call.

## Responsibilities

- `#[pymodule]` / `#[pyfunction]` decorators.
- Python-friendly value conversion (Rust `String`/`Vec<String>` ↔
  Python `str`/`list[str]`; I/O errors → `OSError`).
- Thin translation over `ghd` — no domain logic.

## Exposed functions

| Function | Forwards to |
|---|---|
| `version_banner() -> str` | `ghd::version_banner()` (`ghd <version> (<os>)`) |
| `pr_view(owner, repo, number, fields) -> str` | builds a `ghd::proto::PrViewRequest`, calls `ghd::cli::pr_view_request`, returns the JSON document gh would print |

`pr_view` starts the daemon on first use (via `ghd-core`'s client).
`src/ghd/bindings.py` wraps both and parses the JSON into a `dict`.

If a behavior change needs to land in both the CLI and the Python API,
it goes in `ghd-core` (reached through `ghd`). This crate just exposes
it.

## Build

The soldr PEP 517 backend handles the heavy lifting — no direct
`maturin` invocation anywhere in this repo:

```bash
uv sync              # installs the project via the soldr backend (dev profile)
uv build --wheel     # release-shaped wheel build via the same backend
```

`pyproject.toml` declares `build-backend = "soldr"` and pins this
crate's manifest path via `[tool.maturin]` (soldr drives a pinned
maturin under the hood):

```toml
[tool.maturin]
manifest-path = "crates/ghd-py/Cargo.toml"
module-name = "ghd._native"
python-source = "src"
features = ["pyo3/extension-module"]
```

`pyo3/extension-module` lets the dylib not link against libpython at
compile time. Combined with the workspace's `abi3-py310` feature on
`pyo3`, one wheel per platform covers every supported CPython.

## Module name vs. crate name

- Crate: `ghd-py` (workspace member, name in `Cargo.toml`)
- Library: `_native` (`[lib].name`, becomes the dylib filename)
- Python import: `ghd._native` (set by `module-name` in `[tool.maturin]`)

## No Rust test harness

`[lib] test = false, doctest = false`: `#[pymodule]`/`#[pyfunction]`
code needs a live Python interpreter, which `cargo test -p ghd-py`
cannot provide. `tests/test_bindings.py` (Python side) is this crate's
real test suite, run against the built extension.

## Surface contract

`_native` has NO pure-Python fallback (`PKG-005`):
`src/ghd/bindings.py` imports it directly, unconditionally. If the
extension isn't built, importing the package fails loudly.
