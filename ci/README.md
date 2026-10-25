# `ci/`


Local PR validation uses `python3 ci/local.py act`: a cheap precheck followed
by the complete original workflow through `bosn ci run` and pinned act2.
Selected `--lanes` runs are diagnostics. Preserve remote-selected native
coverage and full release validation; neither is waived by this entry point.

Live GitHub cache trimming runs in the existing serialized `cache-janitor`
job, including bounded trims for open same-repository PRs. It does not run
inside precheck. Bosn reports that job and the live cache-budget audit as
remote-only under GATE-012; their GitHub behavior remains required there.
These reports supply no evidence about the local cache store.

Repo automation. Two structured sub-packages; the release-flow scripts
that used to live here (`build_wheel.py`, `publish.py`) were removed in
zackees/ci.yml#6 round 1 — see below.

## Layout

```
ci/
├── gates/                 # workspace-state checks (run by ./ci.py)
│   ├── __init__.py
│   ├── loc.py             # LOC budget gate (warn 1000 / fail 1500)
│   ├── fmt.py             # soldr cargo fmt --check
│   ├── clippy.py          # soldr cargo clippy -D warnings
│   ├── ruff.py            # ruff check + format --check
│   ├── build.py           # soldr cargo check --workspace (FATAL in `all`)
│   ├── test.py            # soldr cargo test + uv sync (soldr backend) + pytest
│   ├── backend_smoke.py   # uv build --wheel through the soldr backend
│   ├── action_yaml.py     # composite-action structural check
│   └── action_surface.py  # subcommand-vs-binary surface check
├── fast.py                # .github/workflows/ci.yml `fast` job's logic
├── dylint.py               # .github/workflows/ci.yml `dylint` job's logic
├── platform_build.py       # .github/workflows/ci.yml `platform-build` job's logic
├── platform_run.py         # .github/workflows/ci.yml `platform-run` job's logic
├── init.py                 # .github/workflows/ci.yml `init` job's logic (from-zero suite)
├── instantiate.py          # template-instantiation helper the `init` job drives
├── lockfile_changed.py     # cache-maint's Cargo.lock/uv.lock/rust-toolchain.toml diff check
├── plan_profile.py         # ci-pre.yml: derives is-release/profile from plan.flow
├── release_guard.py        # .github/workflows/ci.yml `release-guard` job's logic
├── release.py               # .github/workflows/ci.yml `release-linux-x64`/`release-verify` jobs' logic
├── musl_smoke.py            # .github/workflows/ci.yml `release-musl-smoke` job's logic (musllinux, Alpine container)
├── perf.py                  # .github/workflows/ci.yml `perf` job's logic
├── perf_pyo3_bench.py       # inner PyO3-call timing loop `perf.py bench` runs via the built venv
├── ci_ok.py                # .github/workflows/ci.yml `ci-ok` job's logic
└── hooks/                 # agent-intent guards (run by Claude Code)
    ├── tool_guard.py
    ├── readme_guard.py
    ├── loc_guard.py
    └── check-on-start.py
```

## CI-workflow orchestration (`fast.py` / `dylint.py` / `ci_ok.py`)

Added in zackees/ci.yml#6 round 2 alongside `.github/workflows/ci.yml` +
`ci-pre.yml`. These are NOT gates (they don't run under `./ci.py`) —
they are the Python side of the workflow's `run:` steps, one line per
step (CLAUDE.md rule 6):

- **`fast.py`** — subcommands `build-json`, `rust-test`, `python-test`,
  `wheel-build`, `wheel-install`. Its docstring records the round's
  decisions with evidence: Clippy stays a separate required check (not
  folded into Dylint or `soldr ci-test`'s bundled DAG), and the wheel
  is built with `soldr wheel` (ci.yml#17, #37), which bundles
  `ghd-cli` since soldr 0.9.27 (zackees/soldr#3468). The PEP 517
  backend stays proven by `uv sync` and `release.py sdist-smoke`.
- **`dylint.py`** — reads target triples straight from `ci.toml`'s
  `[platforms]` (not `plan.json`'s `dylint_targets`, which round 1's
  planner narrows to the flow's *build* platform selection — wrong for
  Dylint, which always covers every declared platform; see the module
  docstring). Supports both `--shape sequential` and `--shape
  multi-target` for the D6 invocation-shape measurement. `--print-cross-targets`
  is a second mode the workflow step uses to resolve the Setup-soldr
  `dylint-targets` input before the check pass runs (ci.yml#9,
  setup-soldr v0.9.83+): setup-soldr now prepares rust-std for every
  declared cross target and keys the Dylint foundation/output cache on
  the full target set itself, so this script no longer runs `soldr
  dylint prepare --target` in a loop.
- **`ci_ok.py`** — the `ci-ok` job's one line. Receives the precheck
  job's `plan`/`reuse_json` outputs, `toJSON(needs)`, and
  `toJSON(github.event)` through `env:` (never interpolated into a
  `run:` line), writes them to files, and calls `ci_lint gate --reuse
  ... --event ...`.
- **`platform_build.py`** (round 3) — the `platform-build` matrix job's
  logic: cross-compiles one lane's declared test binaries + wheel (native
  CLI bundled in, via `soldr wheel [--release] --target <triple>` — see
  the module docstring),
  then stages them + a `manifest.json` for `platform_run.py` to download.
  All on `ubuntu-24.04`; nothing here ever runs on the target's own OS.
- **`platform_run.py`** (round 3) — the matching `platform-run` matrix
  job's logic, on that lane's own native runner, with NO Rust toolchain:
  downloads `platform_build.py`'s staged artifact and executes every
  declared test binary directly (setting `CARGO_BIN_EXE_ghd-cli` for
  `ghd-cli:test:cli`, which reads it at runtime), then a clean-venv
  wheel install + smoke, then `tests/integration/` when that suite is
  selected for this lane. Round 5 adds `wheel-install --smoke-out
  <path>`: always writes a `smoke-results/<lane>.json` record, uploaded
  as an artifact only when `is-release`.
- **`plan_profile.py`** (round 5) — one `ci-pre.yml` step: derives
  `is-release`/`profile` (`release`/`dev`) from `ci_lint plan`'s own
  `plan.flow`, so `release-guard`/`release-linux-x64`/`platform-build`/
  `platform-run`'s `--profile` arguments never repeat the `flow ==
  'release' || flow == 'nightly'` ternary inline in a `run:` line
  (GEN-005 flags `&&`/`||` there even inside a `${{ }}` expression).
- **`release_guard.py`** (round 5) — the exact-SHA guard: on
  `workflow_dispatch`, the checked-out commit must equal `inputs.sha` and
  be reachable from `main`; a no-op on every other event (a `[release]`
  PR rehearsal, or `nightly`, has no `inputs.sha` to pin against).
- **`release.py`** (round 5) — linux-x64's sdist + release-profile wheel,
  built through Soldr's manylinux_2_17 cross sysroot (`build --target
  x86_64-unknown-linux-gnu`, same arch as the `ubuntu-24.04` host but the
  controlled sysroot instead of the runner's own newer glibc), its
  native install smoke (`smoke`, writes `smoke-results/linux-x64.json`),
  `glibc-check` (parses the staged wheel's bundled CLI + PyO3 extension
  with `readelf -V`/`objdump -T` for the actual max required `GLIBC_X.Y`
  symbol version and fails over the declared floor — proof from the
  artifact's own bytes, not just the filename tag — see docs/RELEASE.md
  "The glibc floor"), and `collect-wheels` (merges `platform_build.py`'s
  staged cross-platform wheels into one flat `dist/` for `release-
  verify`). `build` writes the sdist through the Soldr PEP 517 backend
  and the wheel with `soldr wheel --release`; `sdist-smoke` builds a
  wheel FROM that sdist and requires the bundled CLI in it (PKG-004).
- **`musl_smoke.py`** (ci.yml#43, musllinux) — `release-musl-smoke`'s one
  line: installs the `release-musl-build`-staged musllinux wheel with
  stdlib `venv`/`pip` (no `soldr`/`uv` -- neither exists in the
  `python:3.13-alpine` container this job runs in) and runs the bundled
  CLI's `--version`, writing the same `smoke-results/<id>.json` shape
  every other platform's smoke step writes. This is the only environment
  that can actually load a musllinux wheel's native extension: a
  glibc-linked CPython cannot dlopen a musl-linked PyO3 extension, and
  pip on a glibc/manylinux host does not consider a musllinux-tagged
  wheel installable at all.
- **`perf.py`** / **`perf_pyo3_bench.py`** (round 5) — `perf.py bench`
  builds a release-profile wheel, installs it into a clean venv, then
  times `ghd-cli --version` startup and (via `perf_pyo3_bench.py`,
  run BY that venv's own python) one PyO3 call, writing a typed
  `BenchmarkFile` JSON (AGENTS.md's dataclass rule). `perf.py
  fetch-baseline` pulls the latest successful `main`/nightly
  `perf-results` artifact through the REST API (stdlib `urllib`,
  `GITHUB_TOKEN`); "no baseline yet" is reported, never a failure.

## Two halves: gates vs. hooks

| Concern                                | Home                       |
|----------------------------------------|----------------------------|
| Runs on every CI cycle                 | `ci/gates/*.py`            |
| Runs only during a Claude/Codex session| `ci/hooks/*.py`            |
| Workspace-wide LOC budget              | `ci/gates/loc.py`          |
| Per-edit LOC budget                    | `ci/hooks/loc_guard.py`    |
| README presence + size                 | `ci/hooks/readme_guard.py` |
| Bare cargo/maturin/uv shape ban        | `ci/hooks/tool_guard.py`   |

If a rule would fire on a `git push` from a terminal the same way it
would fire on a Claude edit, write it as a gate. If it needs to see
what tool is about to run, write it as a hook. See [zccache#835 rule 9](https://github.com/zackees/zccache/issues/835).

## What happened to `build_wheel.py` and `publish.py`

Removed in zackees/ci.yml#6 round 1 (this repository's `ci.toml`
contract, §13):

- **`build_wheel.py`** manually built `ghd-cli` via cargo, drove
  maturin for the PyO3 extension, then hand-patched the resulting
  wheel's zip/RECORD to inject the binary. That whole dance is now
  `[tool.soldr.pep517] bundle-bins` in `pyproject.toml`: the soldr PEP
  517 backend builds and stages `ghd-cli` itself, as part of the
  normal `uv build` / `uv sync` wheel build.
- **`publish.py`** wrapped `twine upload` behind a hand-flipped
  `_ENABLED` guard. Round 5 implements the real replacement, but there is
  still no `ci/publish.py`: the `publish` job in `.github/workflows/
  ci.yml` calls `python3 -m ci_lint publish oidc-check` directly (mints
  the OIDC token, asserts its claims, stops before upload — see
  `docs/RELEASE.md`) — no wrapper script needed on this side.

`ci/gates/*.py` must never call bare `cargo`/`rustc`/`maturin` — every
Rust or wheel command goes through `soldr` (`soldr cargo ...`, `soldr
wheel`, or the soldr PEP 517 backend via `uv build`/`uv sync`).

## Conventions

- Every gate file exposes a single `def run() -> int`.
- Every hook file is invoked as `uv run --no-project --script
  ci/hooks/<name>.py`.
- No multi-line shell in `.github/workflows/ci.yml` — if you can't fit
  a CI step on one line as `./ci.py <gate>`, the logic belongs as a
  gate.
- Host checks (`sys.platform`, `os.name`) are banned in `ci/*.py`; if a
  gate needs one, it imports `ghd.platforms`
  (see `ci/gates/action_surface.py` / `backend_smoke.py` for the
  pattern) rather than branching inline. `ci.toml`'s
  `[allow] platform-code` names this as one of the two allowed
  locations for host-branching code in this workspace.

## Where the dispatcher lives

`ci.py` at the repo root is the PEP 723 dispatcher, directly
executable via its `#!/usr/bin/env -S uv run --no-project --script`
shebang (forge-ci/ghd#15 replaced the former
`ci.sh` bash wrapper with this — no shell script remains). Don't
duplicate that flag combination in CI snippets — always route through
`./ci.py <gate>`.
