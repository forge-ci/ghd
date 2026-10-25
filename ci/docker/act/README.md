# `ci/docker/act/`

The two images behind `python3 ci/local.py act` (zackees/ci.yml#6
section 11). See `ci/docker/README.md` for why there are two, and
`ci/localrun/README.md` for the Python that drives them.

## `Dockerfile.bosn-act` -- the bosn-managed act-driving stack

`debian:bookworm-slim` (pinned by digest) plus:

| Component | Version | Verified sha256 (independently re-checked 2026-09-28) |
|---|---|---|
| `act` (linux/amd64) | 0.2.88 | `1eb9996682dfcc053ac8f3f90f2ec50376f0cdfc229712d82da03d673c63a2b3` |
| `act` (linux/arm64) | 0.2.88 | `94d87738f7ea6650782c8505366c758c99db54cc67bd8c711583478c93305d78` (carried over from zccache's pin, not independently re-downloaded on this host) |
| Docker CLI (amd64 static) | 29.7.2 | `803d433f226db4776e1768fd319fc6c6e4935a456acf84fcc0080818b854bc8f` |
| Docker CLI (arm64 static) | 29.7.2 | `43d143448adf2c2787704e7d7704fd6d62d367a54c5edaef0a3f75509cb0938d` (carried over from zccache's pin) |
| `python3` | Debian bookworm's `python3.11.2-6+deb12u8` | apt, no tarball pin available (see `ci/docker/README.md`) |

Built via `bosn ensure --stack act`: 54s cold on this host (2026-09-28,
mostly `apt-get install` for `ca-certificates curl git python3` and
their dependency chain -- 37.8s of the 54.5s total), effectively instant
warm (bosn's own image-digest cache short-circuits an unchanged
Dockerfile).

Confirmed working versions inside the built image:
`act version 0.2.88`, `Docker version 29.7.2, build a7dcaa6`,
`Python 3.11.2`.

## `Dockerfile.runner` -- the act job-container image (D7 fix)

`catthehacker/ubuntu:act-24.04`, pinned by the registry's **multi-arch
index digest** (`sha256:c58e2b364da03b0c804c7d660f2ecbedf2f221a382b9baa
0b344b0144780ff43`, from `docker buildx imagetools inspect
catthehacker/ubuntu:act-24.04` on 2026-09-28 -- covers linux/amd64,
linux/arm64/v8 and linux/arm/v7, so this Dockerfile needs no `ARG` to
stay portable across host architectures), plus `clang` (Ubuntu 24.04
apt, resolved to `1:18.0-59~exp2` / `clang-18 1:18.1.3-1ubuntu1` on
2026-09-28 -- printed at build time by the Dockerfile's own
`clang --version` step, visible in every build log).

This fixes zackees/ci.yml#6 section 11's D7: PR #1541 in
zackees/clud hit `exec: clang: not found` from setup-soldr's linker
shim on the stock `catthehacker/ubuntu:act-24.04` image. Reproduced
here (`ci/localrun/act_inner.py` builds this image *before* the first
`act` invocation, so the fast/dylint lanes' setup-soldr steps always
have a linker on `PATH`) and confirmed fixed: the build log's final
step prints `Ubuntu clang version 18.1.3 (1ubuntu1)` with no error.

Built by `ci/localrun/act_inner.py::_ensure_runner_image`, tagged
`ghd-act-runner:24.04-clang`, referenced by
`act -P ubuntu-24.04=<tag> --pull=false` (never pushed to any
registry). Measured on this host: **cold ~30-37s** (dominated by the
`clang` apt-get install, ~40 packages), **warm: skipped entirely**
(`docker image inspect <tag>` short-circuits the whole function before
any `docker build` call) -- confirmed by a second `python3 ci/local.py
act --lanes fast` completing its `bosn run --task act-run` round trip
in 3.5s total, versus 61.7s cold.

## What a local run actually exercises today

`python3 ci/local.py act --lanes dylint` runs the real `dylint` job (and
its `ci-pre.yml`-reusable `precheck` dependency) end to end in an
act-managed, clang-fixed container talking to the host Docker engine,
and PASSES (zackees/ci.yml#47, closed). Confirmed 2026-09-29:

| run | wall (`bosn run --task act-run`) | lane execution | result |
| --- | --- | --- | --- |
| cold (fresh runner image + no prior Dylint nightly download) | 526.9s | 455.6s | PASSED |
| warm (runner image + `.ci-lint` volume already warm) | 1607.6s (shared-host contention; see note) | 347.7s | PASSED |

Both runs' `act cache audit` show `disk usage: 0B / budget 9.0GB,
complete index entries: 0` -- nothing was uploaded to (or restored from)
any GitHub Actions cache; every byte is local zccache/Dylint-toolchain
state. "Warm" here means the runner image and the `.ci-lint` checkout
are cached, NOT that the Dylint compile itself is warm -- `act` has no
real `GITHUB_TOKEN`, so every `actions/cache` restore/save the workflow
declares is unreachable under act (same limitation the `precheck` job's
own `ACT-001`/`CACHE-005/006/008` `needs_review` findings already
document), and each job container is `--rm`-disposable, so `soldr`
starts from an empty zccache/target every time. The lane-execution delta
(455.6s -> 347.7s) is entirely the Dylint nightly toolchain component
download being warm on the host's rustup cache; it is not a compile-cache
effect. The much larger total-wall gap on the second run is this shared
host's Docker/network contention at the time (other agents' concurrent
builds), not attributable to this change -- lane-execution time (which
excludes bosn/act startup and is measured independently by
`act_inner.py`) is the comparable number.

Three real, independent defects were reproduced and fixed to get here
(zackees/ci.yml#47):

1. **Linked git worktree breaks `git` inside every job container.**
   `act`'s own Checkout step is a plain `docker cp` of `/work`, not a
   real clone. A linked worktree's `.git` is a FILE (`gitdir: <absolute
   path under the main checkout's .git/worktrees/...>`) that gets copied
   verbatim, but the path it points at was never mounted -- every `git`
   command inside the job container failed ("unable to get git
   revision: repository does not exist"), and `ci_lint`'s tracked-file
   scan silently fell back to a raw filesystem walk that does not
   respect `.gitignore`, scanning `.ci-lint/`'s own fixtures and
   producing spurious findings. Fixed in `ci/localrun/act_orchestrate.py`
   (`_detect_external_git_common_dir`) + `ci/localrun/act_inner.py`: the
   host-side orchestrator detects a linked worktree and bind-mounts the
   main checkout's root into every job container BY IDENTITY (same
   absolute path), so the worktree's absolute gitdir reference resolves
   exactly as it does on the host. A no-op for an ordinary (non-worktree)
   checkout. (Every M2- round's worker-contract worktree hit this --
   likely why #47 had never gone green before.)
2. **The `cache-budget` job's live GitHub API call had no act skip.**
   `ci_lint cache budget` needs `GITHUB_TOKEN`/`GITHUB_REPOSITORY` to
   call the live GitHub Actions cache API; act supplies neither, so this
   job failed outright and took the reusable-workflow-call `precheck`
   job down with it (a reusable workflow's status is the aggregate of
   every job inside it, not just the ones a caller's `needs:` names).
   Fixed with the same `if: env.ACT != 'true'` treatment
   `ci-pre.yml`/`ci.yml`'s other GitHub-API-dependent checkout steps
   already use -- "skipped (local), never treated as passing," the exact
   policy the `precheck` job's own `ACT-001`/`CACHE-005/006/008` findings
   already state in words.
3. **setup-soldr's anonymous release-tag resolution is genuinely
   rate-limit-fragile on a shared host.** `zackees/setup-soldr@...`'s
   `fetchReleaseTagDefault` tries an unauthenticated redirect-based
   lookup first (no REST quota cost), falling back to the REST API
   (60 requests/hour per IP, shared across every process on this
   machine) only on failure. When this machine's anonymous quota was
   already exhausted by concurrent activity, both paths returned
   `GitHub API returned HTTP 403 for zackees/soldr` and the job failed.
   This is a real environment/infra fragility, not a template or
   `ci_lint` defect -- no code change here, it resolved once the shared
   IP's hourly quota rolled over (confirmed: `x-ratelimit-reset` from
   `api.github.com`). `bosn`'s own tooling already documents the
   supported opt-in for a developer who hits this often: `bosn secret
   set github_token --from-gh` (a scoped, zero-permission PAT raises the
   anonymous limit to 5000/hour) -- not exercised by this fix per the
   worker contract's no-secrets rule; left as a documented option for
   whoever owns that decision.
