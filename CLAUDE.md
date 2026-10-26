# CLAUDE.md

Guidance for Claude Code (and any agent) working in this repository.

This is the **canonical hybrid Rust + Python template**, and the fleet
reference repo for the `rust-pypi-app` CI profile
([zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6)).
Practices that land here propagate to every downstream consumer seeded
by `gh repo create --template forge-ci/ghd`, AND
feed back into `ci.toml`'s schema and `ci-lint` in `zackees/ci.yml`.
Bias toward keeping things tight and load-bearing; the leverage is high
in both directions.

## Essential Rules

1. **Always run gates through `./ci.py <gate>`.** Never paste `soldr
   cargo clippy ...` or `uv run python ci/gates/...` directly into a
   command. The `ci/hooks/tool_guard.py` PreToolUse hook blocks bare
   forms and tells you why.
2. **Never call bare `cargo`/`rustc`/`rustup`/`maturin`/`pip` for Rust
   or wheel work — use `soldr`** (`soldr cargo <cmd>`, `soldr wheel`,
   the soldr PEP 517 backend via `uv build`/`uv sync`). This is a fleet
   rule (`RUST-001`/`PKG-003` in `zackees/ci.yml`), not just a local
   convention; `soldr` invocations always pass `tool_guard.py`.
3. **Reserve full `uv run`/`uv sync` (without `--no-project --script`)
   for named build entry points: `./test`, `./install`.** Everything
   else needs the protective flags — see `ci.py` for the rationale.
4. **Every directory must have a `README.md` of ≥ 50 lines.** Enforced
   by `ci/hooks/readme_guard.py` on every edit.
5. **Source files ≤ 1000 lines (warn) / ≤ 1500 (fail).** Enforced both
   on every CI run (`ci/gates/loc.py`) and per-edit
   (`ci/hooks/loc_guard.py`). Split convention:
   `foo.rs` → `foo/mod.rs` + per-domain submodules, with `pub use`
   re-exports in `mod.rs` so the public path is unchanged.
6. **Logic lives in Python under `ci/`; YAML stays thin.** Every future
   CI step is a single line — `run: ./ci.py <gate>` or `run: python3
   ci/<script>.py ...` — never multi-line shell. (There is no
   `.github/workflows/ci.yml` on this branch right now — see "CI status
   on this branch" below.)
7. **`build` is the only fatal gate.** A failing build halts the rest
   of the run because every downstream gate would produce noise
   against an uncompiled tree.
8. **Host checks (`sys.platform`/`os.name`/`cfg(target_os)`) live in
   exactly two places**: `src/ghd/platforms/` (Python)
   and `crates/private/ghd-platform/src/platforms/**` plus its one
   `cfg_select!` selector (Rust). Nowhere else — see `ci.toml`'s `[allow]
   platform-selector` / `platform-code`.

## CI status on this branch

`zackees/ci.yml#6` round 2 added the two workflows round 1 deferred:
`.github/workflows/ci-pre.yml` (`workflow_call`, ≤ 30 s, no tool
installs — precheck's own gate group 2 rules) and
`.github/workflows/ci.yml` (the only entrypoint: `pull_request`,
`push: main`, `schedule`, `workflow_dispatch`). Round 3 added tag-selected
platform lanes and title-edit reuse. Jobs: `precheck` (now also computes
`ci_lint plan --reuse`'s platform-lane matrix + reuse map) → `fast`
(linux-x64 build/unit/wheel smoke; skipped when reused) + `dylint` (one
Linux job, host + every declared cross target; skipped when reused) +
`platform-build`/`platform-run` (tag-selected non-default platforms:
cross-compiled on Linux, executed with no Rust toolchain on each
platform's own runner) → `ci-ok` (the one required check, `if:
always()`, calls `ci_lint gate --reuse ... --event ...`). Every job's
display name carries its `ci_lint`-computed lane digest
(`fast [<digest>]`, `platform-run (<id>) [<digest>]`), which is also how
title-edit reuse matches an already-green job. `./ci.py all` still runs
locally and is what an agent should run before pushing — see "Commands"
below for the exact local precheck command.

## `ci.toml`

The repo's CI contract, checked by `ci-lint` (`zackees/ci.yml`, pinned by
commit SHA — see `ci.toml`'s `linter` field, which MUST match the SHA
`.github/workflows/ci-pre.yml` and `ci.yml` check out into
`.ci-lint`, rule `CT-004`). It declares the six supported platforms, the
Rust workspace's public/private crate split and test-binary budget, the
Python packaging shape (soldr backend, `abi3-py310`, `bundle-bins`),
suites, flows, tags, and cache families. It does not generate YAML — it
bounds what the workflow may do and how it plans each run.

## Commands

```bash
./install                # verify uv + soldr + pinned toolchain (no wheel build)
./ci.py fmt               # one gate
./ci.py all               # every gate, continue past failures
./ci.py --list             # show registered gates
./test                    # soldr cargo test + uv sync (soldr backend) + pytest
./lint                    # convenience: fmt + clippy + ruff
```

## CI

### Local loop: `ci/local.py` (bosn -> act, zackees/ci.yml#6 §11)

```bash
python3 ci/local.py precheck                  # ~0.5s warm; ~1-2s cold (clones .ci-lint/ once)
python3 ci/local.py act                       # precheck, then the complete PR workflow via Bosn -> act2
python3 ci/local.py act --lanes fast          # selected-job diagnostic
python3 ci/local.py act --title "[ci-full] …" # exercise a different tag selection
```

`precheck` wraps the exact command below, resolving `<ci.yml checkout>`
to a gitignored `.ci-lint/` clone/fetch of `ci.toml`'s `linter` pin
(no-op once already at that SHA — see `ci/localrun/ci_lint_checkout.py`):

```bash
PYTHONPATH=<ci.yml checkout> uv run --no-project --with pyyaml python3 -m ci_lint precheck --repo . --local
```

Run `python3 ci/local.py precheck` before every push that touches a
workflow, `ci.toml`, `Cargo.toml`, `pyproject.toml`, `bosn.toml`, or
`action.yml` — it is also the agent's PostToolUse hook on those paths
and its Stop hook (`.claude/settings.json` →
`ci/hooks/local_precheck_guard.py`), so this normally runs
automatically; run it by hand to iterate faster than the hook's
edit-triggered cadence.

`act` requires a published Bosn CLI with its supervised CI runner on PATH.
The default calls the pinned shared `ci-lint local-gate run`. Its declaration
in `local-gate.toml` calls
`bosn ci run --workflow .github/workflows/ci.yml --trigger pr --wait --json`
without a job filter, so precheck, fast (build, tests and installed wheel),
Dylint and CI OK run as one original workflow. The shared tool validates
qualified execution evidence and writes tree/parent/input-bound commit
trailers only after success. Repeating an unchanged qualified local run may
reuse its fresh lane receipts. Start from a clean committed tree.

`ci-attestations.yml` maps only fast and Dylint checks. The hosted verifier
and shared CI OK aggregator independently check the immutable PR-base policy
and head proof before crediting a skipped job. Remote cache maintenance and
its reusable precheck caller remain required. Title tags, forks, audit
samples, missing proof and changed trust surfaces force remote execution;
main pushes and release dispatches never use local-attestation skips.
Unattested developer heads remain allowed in the pilot and run remotely
(`mode = "shadow"`). The independent end-to-end pilot is being qualified in
[zackees/ci.yml#362](https://github.com/zackees/ci.yml/issues/362). Bosn owns the pinned act2
binary, frozen Git snapshot, isolated Docker engine, action/cache storage,
logs and cleanup. The legacy host-socket `act-run` stack is no longer used.
`--lanes` requests selected-job diagnostics, never a full PR proof.
`--title` is passed as `--pr-title` to Bosn's event adapter. Native coverage
selected by that title remains required; unsupported runners must fail
rather than being reported as passed. Ordinary PRs retain their existing
coverage and release validation remains the original full release workflow.

The `dylint` job's own logic lives in `ci/dylint.py` (host + every
declared cross target, one Linux job — `soldr dylint prepare --target T`
then `soldr cargo dylint`, never a bare `cargo`/`cargo-dylint` call). The
`fast` job's logic lives in `ci/fast.py`. `.github/actions/soldr/` is the
ONLY `zackees/setup-soldr` call site (`ci.toml`'s
`[allow] setup-soldr.only-in`) — see its README before adding a lane that
needs a new setup-soldr input.

For the full design rationale see
[zackees/zccache#835](https://github.com/zackees/zccache/issues/835)
(the gates/hooks/entry-point shape) and
[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) (the
crate layout, packaging, and `ci.toml` contract).

## Hooks vs Gates

| Concern                                | Home                       |
|----------------------------------------|----------------------------|
| Repo-state checks (`git push` catches it) | `ci/gates/*.py`         |
| Agent-intent checks (only during sessions)| `ci/hooks/*.py`         |
| LOC budget across the workspace        | `ci/gates/loc.py`          |
| LOC growth on this edit                | `ci/hooks/loc_guard.py`    |
| README presence + size                 | `ci/hooks/readme_guard.py` |
| Bare cargo/maturin / unsafe `uv run` shape | `ci/hooks/tool_guard.py` |
| Contract precheck on `.github/**`/`ci.toml`/`Cargo.toml`/`pyproject.toml`/`bosn.toml`/`action.yml` edits, and on every Stop | `ci/hooks/local_precheck_guard.py` |

If a rule would fire equally well on a `git push` from a terminal as
from a Claude edit, write it as a gate. If it needs to know what tool
is about to run, write it as a hook.

## Repo Shape

- `crates/template` — the public amalgam (published crate): `pub use`
  re-exports only, feature-gated `dep:` wiring
- `crates/ghd-cli` — the `ghd-cli` binary, bundled into the
  wheel via soldr
- `crates/ghd-py` — `PyO3` `_native` extension crate
- `crates/private/` — `publish = false` crates, each compiled and
  tested exactly once: `ghd-core` (domain logic),
  `ghd-daemon` (an optional feature, as a crate),
  `ghd-platform` (the host facade)
- `src/ghd/` — Python package: bindings, package
  metadata, `platforms/` (host facade)
- `ci/` — automation (see `ci/README.md`)
- `action.yml`, `action/cleanup/action.yml` — composite action
  contract; every `run:` step is one line calling a script under
  `action/`

## Working Rules

- Grow `ghd-core` first; expose it through `template` (the
  amalgam), then `ghd-cli`/`ghd-py` pick it up automatically.
  Never let the CLI and the bindings diverge on core behavior.
- The wheel exposes `ghd._native` (PyO3, no pure-
  Python fallback) AND ships the soldr-built `ghd-cli[.exe]` as a
  raw wheel script at `ghd-<ver>.data/scripts/`
  (`[tool.soldr.pep517] bundle-bins`). Pip extracts that directly into
  the venv's `Scripts/` / `bin/` on install — no Python shim sits in
  front of it.
- When changing user-visible commands, update [README.md](./README.md),
  [UPDATE.md](./UPDATE.md), [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md),
  `ci.toml` (if the change affects platforms/suites/packaging), and any
  affected `ci/gates/<name>.py`.

## Design principles (fleet directives, zackees/ci.yml#163)

Agent and review guidance from zackees/ci.yml `docs/policy-rust.md` ("Design principles, narrowed"). Not enforced by ci-lint; no rule ID.

1. **Search before you add.** Before adding a resolver, detector, predicate, guard or path helper, `rg` for an existing one. Reuse it, or say in the PR why it does not fit.
2. **Conflicting implementations get a research issue, not an inline winner.** If the same concept is implemented more than once with observably different behaviour and fixing it needs a design decision, file `research: N conflicting implementations of X -- which is canonical?` listing every copy (`file:line`) and what the difference does to real input; cross-reference it and carry on with your task. Copies that behave identically are a refactor, not an issue.
3. **Keep behaviour changes and refactors apart.** Separate commits, preferably separate PRs, never one mixed diff.
4. **Refactor first only when you would grow a giant function.** If a fix would add more than 50 lines to a function already over 200 lines, extract first in a separate behaviour-preserving change with characterization tests written first (RED -> GREEN). Otherwise fix it where it belongs; no unrequested restructures.
5. **Function and file size follow the complexity ratchet (`RUST-018`).** New functions stay under clippy's `too_many_lines` (100) and `cognitive_complexity` (25). A function carrying an `#[expect]` must not grow; delete the `expect` once it is back under. Never split a file with `include!` fragments to dodge a length ceiling (Essential Rule 5's LOC budget included). Before splitting a hot file, check `git branch -a --contains` for in-flight branches -- a split is a rename and conflicts destructively.
6. **Fail fast, and delete before you abstract.** Reject invalid input at the boundary with a typed error; never fall back silently to a degraded path. Prefer deleting code to adding a layer. No new trait until there are two real implementations (a test double does not count).
7. **Existing boundaries beat generic DRY.** Platform modules (Essential Rule 8's host facade), Dylint-enforced layering, crate boundaries and documented deliberate copies win over removing duplication. Do not copy code across a crate boundary to get around it -- say the boundary hurts. An undocumented copy is a finding under 2; a documented one is not.

## Where to ask questions

- Gates/hooks/entry-point design → [zackees/zccache#835](https://github.com/zackees/zccache/issues/835)
- Crate layout, packaging, `ci.toml` contract → [zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6)
- Architecture → [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)
- Release flow → [docs/RELEASE.md](./docs/RELEASE.md)
- Linting policy → [LINTING.md](./LINTING.md)
