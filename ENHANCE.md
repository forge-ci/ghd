# Enhancement Notes

When growing the scaffold, the load-bearing decisions are:

## Where new logic goes

- **Reusable behavior** → `crates/private/ghd-core`, reached
  through the public `crates/template` amalgam. Both the CLI binary
  and the PyO3 bindings depend on `template`, never on `ghd-core`
  directly; growth in `ghd-core` propagates to both consumers for
  free once re-exported.
- **An optional capability** → a new crate under `crates/private/`,
  wired into `crates/template`'s `[features]` table as plain `dep:`
  wiring (see `crates/private/ghd-daemon` for the pattern). Never a
  `#[cfg(feature = ...)]` branch inside `ghd-core`'s own logic.
- **CLI commands** → `crates/ghd-cli`. Subcommands/flags here
  become part of the composite action's surface contract — adding one
  means updating `action/smoke_test.py` and letting
  `ci/gates/action_surface.py` verify the binary still exports it.
- **Public Python API** → `src/ghd/bindings.py`.
  Keep the wrapper thin: each function should be a near-1:1 reflection
  of the underlying `_native` call, with type annotations and a
  one-line docstring.
- **A host-specific check** (Python OR Rust) → the platform facade,
  never inline. Python: `src/ghd/platforms/`.
  Rust: `crates/private/ghd-platform/src/platforms/**`, reached
  via `crate::platform::*`. See `ci.toml`'s `[allow] platform-selector`
  / `platform-code` — these are the only two allowed locations.
- **Native Rust boundary** → `crates/ghd-py/src/lib.rs`. PyO3
  decorators belong here, not in `ghd-core`.
- **CI logic** → `ci/gates/<name>.py`. Never in workflow YAML (there
  is no workflow on this branch yet — see `docs/ARCHITECTURE.md`) and
  never bare `cargo`/`maturin` — go through `soldr`.

## Where new infrastructure goes

- **New gate** → `ci/gates/<name>.py` exposing `def run() -> int`,
  registered in `ci.py::GATE_ORDER`.
- **New hook** → `ci/hooks/<name>.py` reading JSON from stdin,
  wired in `.claude/settings.json`.
- **New named build entry point** → a Python script at the repo root
  with shebang `#!/usr/bin/env -S uv run --no-project --script` (PEP
  723 header); add its name to
  `ci/hooks/tool_guard.py::BUILD_ENTRY_POINTS` so the hook knows it's
  allowed to use full `uv run`/`uv sync`. No `.sh`/`.ps1`/`.bat`/`.cmd`
  files or bash-shebang scripts are allowed anywhere in the repo
  (`ci_lint`'s `GEN-005` shell budget enforces this with zero
  exceptions).

## Invariants to preserve

- `ghd-cli` and `ghd-py` never diverge on core behavior —
  both depend on the same `template` amalgam.
- There is no Python CLI shim. `ghd-cli[.exe]` ships as a raw
  wheel script at `ghd-<ver>.data/scripts/`,
  staged by the soldr backend's `bundle-bins`; pip drops it straight
  into the venv's `Scripts/` / `bin/` on install with no Python
  wrapper. Adding `[project.scripts]` back would re-introduce the
  Windows `os.execv` race fixed in #7 — don't.
- `_native` has no pure-Python fallback. `bindings.py` imports it
  unconditionally.
- The composite `action.yml` only references subcommands that exist
  in `ghd-cli --help`. `ci/gates/action_surface.py` checks this.
- Every `run:` step in `action.yml` / `action/cleanup/action.yml` is
  one line calling a script under `action/` — no inline shell logic.
- Private crates (`crates/private/*`) never grow a `[features]` table
  or an optional dependency of their own.

## When in doubt

Read [CLAUDE.md](./CLAUDE.md) for the essential rules, then
[UPDATE.md](./UPDATE.md) for the change checklist, `ci.toml` for the
current CI contract, then the relevant gate's docstring for why the
check exists.
