# Linting Policy

The CI gates that lint the repo, in canonical order.

## Rust

Two gates, both via `./ci.py`, both through `soldr` — never bare
`cargo` (see `ci.toml`'s `[allow] tools` and
[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6)):

| Gate              | Command                                                                |
|-------------------|--------------------------------------------------------------------------|
| `./ci.py fmt`     | `soldr cargo fmt --all -- --check`                                     |
| `./ci.py clippy`  | `soldr cargo clippy --workspace --all-targets --locked -- -D warnings` |

Both run locally today via `./ci.py all`. There is no
`.github/workflows/ci.yml` on this branch right now — see
`docs/ARCHITECTURE.md` — a later round wires these into a workflow
planned by `ci.toml`. Failing format is fixable with `soldr cargo fmt
--all`; failing clippy points at a real issue (a denied warning, an
unused-result, etc.).

## Python

One gate covering lint + format:

| Gate              | Command                                                                |
|-------------------|--------------------------------------------------------------------------|
| `./ci.py ruff`    | `ruff check` + `ruff format --check` over `src tests ci action ci.py lint test install` |

`ruff` is provisioned at script-time via `uv run --no-project --with
ruff==<pin>`, so it never triggers a soldr-backend wheel build to lint
a few `.py` files.

## LOC Budget

| Gate              | Threshold                          |
|-------------------|--------------------------------------|
| `./ci.py loc`     | warn > 1000, fail > 1500 (per file)|

Split convention printed on every failure:
`foo.rs` → `foo/mod.rs` + per-domain submodules, with `pub use`
re-exports in `mod.rs` so the public path is unchanged. The
per-edit half lives in `ci/hooks/loc_guard.py`.

## Agent Maintenance Rule

If you add new tooling:

- write the gate at `ci/gates/<name>.py` with `def run() -> int`,
  using `soldr` for any Rust/wheel command (never bare
  `cargo`/`maturin`)
- register it in `ci.py::GATE_ORDER`
- update [README.md](./README.md) and this file
- explain in the gate's docstring why this gate belongs here instead
  of the language-native default
- once a future round adds `.github/workflows/ci.yml`, also add a step
  there — none exists on this branch yet

## What's intentionally NOT here (yet)

- **`mypy` / `pyright`.** Type-checking the thin Python surface adds
  more noise than signal for a template; downstream consumers that
  grow real Python should add a gate. Pattern: `ci/gates/pyright.py`
  with `subprocess.run(["uv", "run", "--no-project", "--with",
  "pyright", "pyright", "src", "tests"])`.
- **Cross-target `soldr lint rust/all` / Dylint as a gate.** `ci.toml`
  declares the shape (`[lint.dylint]`) this template is building
  toward — one Linux job, one check-only pass per declared platform,
  no baseline — but it isn't wired into `ci/gates/` yet. See
  [zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6)'s
  Dylint use case for the plan and the caching/timing work still
  needed before it lands here.
