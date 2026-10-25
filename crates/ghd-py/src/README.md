# `crates/ghd-py/src/`

Source for the PyO3 extension module `ghd._native` (`[lib] name =
"_native"`, `crate-type = ["cdylib"]`). One file.

## What's here

| File | Contents |
|---|---|
| `lib.rs` | Two `#[pyfunction]`s and the `#[pymodule] fn _native`. |

## The functions

```rust
#[pyfunction]
fn version_banner() -> String;          // ghd::version_banner()

#[pyfunction]
fn pr_view(owner: String, repo: String, number: u64, fields: Vec<String>)
    -> PyResult<String>;
```

- `version_banner` forwards to `ghd::version_banner()`:
  `ghd <version> (<os>)`.
- `pr_view` builds a `ghd::proto::PrViewRequest` and calls
  `ghd::cli::pr_view_request`, the same round trip `ghd pr view` makes.
  On success it returns the response's `json` bytes as a `str` (lossy
  UTF-8): the document gh would print. Any `io::Error` — daemon could
  not start, daemon returned an `error` (bad field, GitHub failure) —
  becomes a Python `OSError`.

The `_native` module registers both. Their Python signatures are
mirrored in `src/ghd/_native.pyi`, and `src/ghd/bindings.py` wraps them
(`pr_view("owner/repo", n, fields)` → `dict`).

## Why so thin

The extension is a translation layer: Rust types to Python types and
errors to exceptions. Anything with behavior goes in `ghd-core` so the
CLI and Python share it. This crate depends on `ghd` with default
features (no `daemon`): the extension never runs the server; when no
daemon answers, `ghd-core`'s client spawns the `ghd` binary (from
`$GHD_EXE` or PATH) to do it.

## Why it matters for latency

A Python caller such as `pr_merge_watch` gets one loopback HTTP/1.1
request per read with no subprocess and no async runtime in its
process. A warm read is answered from the daemon's cache.

## Adding a function

1. Make sure the behavior exists in `ghd-core` and is re-exported by
   `ghd` (`crates/ghd/src/lib.rs`).
2. Add a `#[pyfunction]` here that only converts arguments/results.
3. Register it in `_native` with `module.add_function(...)`.
4. Add it to `src/ghd/_native.pyi` and wrap it in `src/ghd/bindings.py`.
5. Cover it in `tests/test_bindings.py`.

## Testing

No Rust harness (`test = false`, `doctest = false`): PyO3 code needs a
live interpreter. `tests/test_bindings.py` exercises the built
extension; build it with `uv sync`.
