# `.github/actions/soldr`

The ONLY sanctioned `zackees/setup-soldr` call site in this repository.
`ci.toml`'s `[allow] setup-soldr.only-in` names this path; `ci-lint`
precheck group 4 (`TOOL-*`) rejects a direct `zackees/setup-soldr@…` step
anywhere else in `.github/workflows/**` — see
[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) §11 rule 3
("One sanctioned setup-soldr call site").

## Why a wrapper, not a direct call per job

Every lane in `ci.yml` (`fast`, `dylint`) needs `setup-soldr`, and every one
of them must pass the same three mandatory inputs
(`solo-toolchain-cache: false`, `cook-delta: false`, `save-cache: auto`) —
see `ci.toml`'s `[allow] setup-soldr.require`. Copy-pasting the call site
per job is exactly how zccache#1760 happened: a workflow edit re-enabled
`solo-toolchain-cache` on one call site while the others stayed compliant,
and nothing caught the drift until the cache footprint audit fired days
later. One wrapper makes the mandatory inputs structurally impossible to
omit — they are hardcoded in `runs.steps[0].with`, never exposed as this
action's own `inputs:`, so a caller cannot override them even by accident.

## Pinning

- `zackees/setup-soldr@v0` follows the fleet's sanctioned first-party
  moving ref. Its promotion is gated by setup-soldr's own downstream
  validation; SEC-004 expressly permits this ref. Other action references
  remain SHA-pinned. See [the Rust policy's pinning exception](https://github.com/zackees/ci.yml/blob/main/docs/policy-rust.md#reconciling-sec-004-with-float-by-default).
- The wrapper continues to request every declared Dylint cross target and
  preserves its fixed cache/save inputs. A change in the upstream action
  must pass the complete local PR workflow and ordinary remote checks.
- No `version:` input is set for the `soldr` binary itself
  ([zackees/ci.yml#18](https://github.com/zackees/ci.yml/issues/18),
  `RUST-013`): it floats on `setup-soldr`'s own latest-release resolution,
  matching `pyproject.toml`'s floor-only `requires = ["soldr>=0.9.26"]` —
  the PEP 517 backend's own requirement — so the CLI used by
  `./ci.py`/CI steps and the backend used by `uv build`/`uv sync` always
  resolve to the same current Soldr release, without a stale exact pin
  in either place.

## Dylint cache fix validation (setup-soldr#538, v0.9.81)

Before `v0.9.81`, setup-soldr compared Dylint's success marker and looked
up Dylint output paths using the short, requested toolchain channel (e.g.
`nightly-2026-05-28`), while soldr's own Dylint plan is host-qualified
(e.g. `nightly-2026-05-28-x86_64-unknown-linux-gnu`). The exact-string
mismatch meant `dylint-cache` and `dylint-output-cache` were never saved
on a successful Dylint run — every run recomputed the lint from scratch.

- **Pre-fix warm baseline** (`v0.9.80`, PR run
  [36495222596](https://github.com/forge-ci/ghd/actions/runs/36495222596)):
  `dylint-cache: no matching successful Dylint marker - skipping save`,
  `dylint-output-cache: Dylint did not complete successfully - skipping
  save`. `dylint (all declared targets)` job wall time 2m33s; 6 Dylint
  passes totaling 91.7s of lint work.
- **Post-fix, first `main` push after ingesting `v0.9.81`** (run
  [36501247176](https://github.com/forge-ci/ghd/actions/runs/36501247176)):
  `dylint-cache: saved id=8239364944
  key=setup-soldr-dylint-v2-linux-x64-x86_64-unknown-linux-gnu-d153183e2b438407-dylint`
  (668,506,333 bytes uploaded), `dylint-output-cache: saved
  id=8239367155 key=setup-soldr-dylint-output-v1-linux-x64-19a07e71336de272`
  (74,568,687 bytes uploaded).
- **Post-fix, this PR's own run** (run
  [36501659725](https://github.com/forge-ci/ghd/actions/runs/36501659725),
  a *new* commit): `dylint-cache: hit=true matched=...-dylint` (the
  toolchain/driver-scoped foundation cache — exact hit, as expected for
  any commit using the same Dylint toolchain). `dylint-output-cache:
  hit=false` — **expected**, not a regression: `dylintOutputHash`
  (`src/lib/resolve-setup.ts`, unchanged by #539) has always included
  `source_revision: githubSha`, so this per-commit-scoped cache only
  hits when the exact same commit re-runs. Job wall time 2m05s.
- **Post-fix, same-commit re-run** (`gh run rerun` of run 36501247176 on
  its unchanged commit `021a5971ae88569d6b965b7acdb2ee3003eb5f99`):
  `dylint-cache: hit=true` and `dylint-output-cache: hit=true`, both
  logging `exact hit - skipping save`. Job wall time 1m58s, vs. the
  pre-fix warm baseline's 2m33s — about 35s (23%) faster wall time on a
  fully warm, same-commit run now that both caches actually restore.

## Cross-commit `dylint-output-cache` fix validation (setup-soldr#540, v0.9.82)

`v0.9.81` fixed the success-marker identity mismatch above, but
`dylint-output-cache`'s own key (`dylintOutputHash`, `src/lib/
resolve-setup.ts`) still included `source_revision: githubSha`, so it was
an exact-key miss on every **new** commit even with an unchanged Dylint
toolchain, lint library and `Cargo.lock` — see run 36501659725 above
(`dylint-output-cache: hit=false`, **expected** under `v0.9.81`, not a
regression). `v0.9.82` (setup-soldr#541) drops the commit from the exact
key and adds a `restore-keys` fallback that drops only the `Cargo.lock`
component. Validated on branch `r2f-output-key` (deleted after this
ingestion; its cache entries were deleted too), pinned to setup-soldr PR
#541's head SHA:

- **Cold, new key shape** (run
  [36504380511](https://github.com/forge-ci/ghd/actions/runs/36504380511),
  job `109202536282`): `dylint-output-cache: key=setup-soldr-dylint-output-v2-linux-x64-2f3052f3bf6820ab-a87d2454829c34f1
  hit=false` -> `dylint-output-cache: saved id=8240368805
  key=setup-soldr-dylint-output-v2-linux-x64-2f3052f3bf6820ab-a87d2454829c34f1`
  (71,397,993 bytes). Job wall time 1m24s (`dylint-cache` foundation
  already hit).
- **Same key, unchanged `Cargo.lock`, one Rust line changed** in
  `crates/private/ghd-core/src/lib.rs` (run
  [36504625315](https://github.com/forge-ci/ghd/actions/runs/36504625315),
  job `109203913679`): **the exact same key** as the cold run above now
  restores with `hit=true matched=setup-soldr-dylint-output-v2-linux-x64-2f3052f3bf6820ab-a87d2454829c34f1`
  — an **exact hit across two different commits**. The log still shows
  `Checking ghd-core v0.1.0 (.../crates/private/ghd-core)`,
  proving the changed crate was genuinely rechecked against the restored
  tree rather than silently skipped. Job wall time **54s** — 2.3x faster
  than the pre-2F, new-commit MISS baseline (run 36501659725, 2m05s) and
  well under the `[lint.dylint].budget.warm` figure this repo tracks.
- **Correctness: a restored cache must not hide a new violation.** A
  `#[cfg(windows)]` added to the same file, outside the platform facade
  (run [36505737115](https://github.com/forge-ci/ghd/actions/runs/36505737115),
  job `109206895144`, deliberately split across lines so the cheap static
  `LAYOUT-001` precheck scan would not also catch it, isolating the
  Dylint-specific signal): `dylint-output-cache` again restored via an
  **exact hit**, then Dylint **failed**: `error: host-platform selection
  outside the ghd-platform boundary: host cfg \`cfg(windows)\`` at
  `crates/private/ghd-core/src/lib.rs:16:1`,
  `#[deny(platform_boundary)] on by default`. Reverted immediately after.

## Inputs

| Input | Forwards to | Used by |
|---|---|---|
| `dylint` | `dylint` | the `dylint` job: resolves the requested nightly, runs `soldr dylint prepare`, exports `SOLDR_DYLINT_SUCCESS_MARKER` / `SOLDR_DYLINT_CONFIGURED_*` |
| `dylint-toolchain` | `dylint-toolchain` | the `dylint` job: pinned to `nightly-2026-05-28` -- this workspace's auto-mapped nightly (`nightly-2026-02-28`) has no catalogued driver asset |
| `prebuild-deps` | `prebuild-deps` | the `dylint` job sets `none`: `soldr cook`'s stable-toolchain deps can't warm a nightly Dylint compile |
| `cross-target` | `cross-targets` | a cross-target Dylint or cross-build pass; exactly one triple per call |
| `ci-tests` | `ci-tests` | the `fast` job's `soldr ci-test` step: declares the linked-test-product cache exclusion |
| `prebuild-deps-flags` | `prebuild-deps-flags` | MUST match the job's actual profile — empty for dev/check/dylint, `--release` for the wheel build |
| `verify-compile-cache` | `verify-compile-cache` | set `error` only on the job proving D8 (does `dylint-driver` compile through zccache) |
| `compile-cache-stats` | `compile-cache-stats` | diagnostic verbosity |
| `cache-key-suffix` | `cache-key-suffix` | gives two jobs on the same target distinct cache identities when they must not share one key |

## Reading the Dylint success-marker contract (D4)

When `dylint: true`, `setup-soldr` exports `SOLDR_DYLINT_SUCCESS_MARKER`
(a file path) and `SOLDR_DYLINT_CONFIGURED_TOOLCHAIN` /
`_RUSTC_RELEASE` / `_RUSTC_COMMIT_HASH` as job-scoped environment
variables — they are visible to every later step in the same job, not just
this action's own steps (composite actions export via `GITHUB_ENV`). The
post step's `dylint-output-cache` save gate compares the file at
`$SOLDR_DYLINT_SUCCESS_MARKER`, trimmed, against this action's
`dylint-cache-identity` output (`<nightly>|<rustc-release>|<rustc-commit>`)
character for character. `soldr cargo dylint` / `soldr dylint prepare
--target` are expected to write that file on a successful run using the
short, `SOLDR_DYLINT_CONFIGURED_*`-supplied identity — see
`ci/dylint.py` and the `dylint` job in `ci.yml` for what happens when they
don't (ci.yml#1's short-vs-host-qualified nightly mismatch).

## What NOT to do

Do not add a second `uses: zackees/setup-soldr@…` step anywhere else in
this repository. If a lane needs a setup-soldr input this wrapper doesn't
forward, add it here as a new pass-through input — never as a parallel
call site.
