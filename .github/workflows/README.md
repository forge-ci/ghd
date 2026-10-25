# `.github/workflows/`

Exactly two files, per [zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6)
§2 ("Two workflows: precheck gates everything") — `ci-lint` precheck
(`GEN-001`/`GEN-008`) rejects a third. Added in round 2, replacing the
`ci.yml` round 1 deleted here: an 8-entry native matrix with bare
`cargo`/`maturin`, no `setup-soldr`, no `timeout-minutes`,
`continue-on-error: true` on 8 of 9 gate steps, and a retired `macos-13`
runner that queued 24h on every run (0 of 27 historical runs ever
succeeded — see round 1's PR body for the evidence).

## The two files

- **`ci.yml`** — the only workflow with `pull_request`
  (`opened`/`synchronize`/`reopened`/`edited`/`closed` -- `closed` runs
  only `ci-pre.yml`'s `cache-janitor`), `push: main`,
  `schedule`, and `workflow_dispatch` triggers. Top-level
  `permissions: contents: read` and a `concurrency` group per PR number
  or ref (cancel-in-progress only for `pull_request`). Its `precheck`
  job calls `ci-pre.yml` via `workflow_call`; `fast` and `dylint`
  both `needs: precheck` and run in parallel; `ci-ok` (`if: always()`)
  needs all three and is the one required check.
- **`ci-pre.yml`** — `workflow_call` only, `ubuntu-24.04`,
  `timeout-minutes: 5`, no tool installs (system `python3` + preinstalled
  `yq`). Checks out this repo and `zackees/ci.yml` at the SHA `ci.toml`'s
  `linter` field pins (`.ci-lint/`), then runs ONE step:
  `python3 -m ci_lint precheck --repo . --plan-out plan.json
  --github-output`. Emits `plan.json`'s contents plus convenience outputs
  (`platforms_json`, `cross_platforms_json`, `suites_json`, `mergeable`)
  that `ci.yml`'s jobs read through `fromJSON` — none of them contain
  their own selection logic. No workflow-level `concurrency`
  (zackees/ci.yml#23, GEN-015). Two more jobs: `cache-janitor`
  (`ci_lint cache janitor`; every push/schedule/dispatch and PR close;
  the only job with a lock, `group: cache-janitor`,
  `cancel-in-progress: false`; nothing `needs:` it) and `cache-budget`
  (`ci_lint cache budget`, the budget verdict: hard on main, warn-only
  on a PR unless its own `pr-<N>` entries caused the breach). Every
  PR-context cache save carries the plan's `cache_key_pr` (`pr-<N>`).

No other workflow file is added. `ci.toml` (repo root) is the contract
these two files implement — platforms, suites, flows, tags, and cache
families are declared there, not hand-written into YAML.

## Local equivalent

```
./ci.py all
PYTHONPATH=<ci.yml checkout> uv run --no-project --with pyyaml python3 -m ci_lint precheck --repo . --local
```

The first runs the same gate set `fast`'s steps call (`ci.py::GATE_ORDER`
still exists and is still the developer entry point — `fast.py`'s steps
are the CI-specific shapes: JSON-message capture, wheel build+install).
The second is the same precheck `ci-pre.yml` runs, and is this
repo's agent Stop-hook / pre-push gate (zackees/ci.yml#6 §11). See
`ci.toml`'s `[local]` table for the `bosn → act` local-runner story.

## Step shape

Every `run:` step is one line — no multi-line shell, no
`shell: pwsh|cmd|powershell`. Any content that could contain untrusted
text (a PR title, `toJSON(needs)`) is passed through `env:`, never
interpolated directly into the `run:` string (script-injection rule —
see `ci-pre.yml`'s precheck step and `ci.yml`'s `ci-ok` job).
Actions are pinned by full commit SHA with a version comment; every job
that can declare one has `timeout-minutes`; runner labels are explicit
(`ubuntu-24.04`, never `-latest`). `CARGO_INCREMENTAL: "0"` on every
compile-bearing job — `incremental/` is on `ci.toml`'s `[cache] never`
list and must never be cached, even though `.cargo/config.toml` enables
it for local dev.

## Adding a workflow

Don't. Only `ci.yml` and `ci-pre.yml` are ever allowed in this
directory — see `CLAUDE.md` and
[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6). A new
lane goes into `ci.toml` (`[suites]`/`[flow.*]`/`[tags]`) and, if it
needs new orchestration logic, a new `ci/*.py` script the existing jobs
call — not a new `.github/workflows/*.yml` file.
