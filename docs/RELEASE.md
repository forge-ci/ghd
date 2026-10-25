# Release Flow

## Status: OIDC mock publish implemented (zackees/ci.yml#6 round 5)

[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) round 1
removed `ci/publish.py` (the guarded `twine upload` wrapper) and
`./publish` along with it — that flow depended on `ci/build_wheel.py`'s
manual wheel-injection step, which is also gone (the soldr PEP 517
backend bundles `ghd-cli` into the wheel directly via `[tool.soldr.
pep517] bundle-bins`). Round 5 implements the real replacement: `ci.toml`
`[publish]` (`auth = "oidc"`, `mode = "mock"`) OIDC-trusted publish that
proves the identity and stops before any upload — no tokens, ever, and
there is still no `ci/publish.py`: `.github/workflows/ci.yml`'s
`publish` job calls `python3 -m ci_lint publish oidc-check` directly.
Don't reintroduce a `twine`-based path — this profile is OIDC-only.

### The release graph, end to end

| Trigger | Flow | What runs |
| --- | --- | --- |
| `[release]` in a PR title | `flow.release`, `publish = "rehearsal"` | Every platform's release-profile wheel + the linux-x64 sdist, every native install smoke, `release-verify` (`ci-lint release verify`). **No** `publish` job, **no** `id-token` — a rehearsal only (issue #6 §7). |
| `workflow_dispatch` with `sha` on `main` | `flow.release`, `publish = "mock"` | Same graph, plus `release-guard` (the exact-SHA guard: HEAD must equal `inputs.sha` and be reachable from `main`, checked before any build) and `publish` (mints the OIDC token, asserts its claims — repository/ref/environment/event_name/workflow_ref/aud — prints PASS/FAIL per claim, then "mock publish: stopping before upload"; the raw token is never printed or logged). |
| `schedule` (nightly) | `flow.nightly` (`extends = "release"`) | Same release-profile wheel matrix (warms every target's caches, per issue #6 §6), but `publish = "none"` — no `release-verify`, no `publish` job. |

`release-linux-x64` builds the one platform `plan.platform_lanes`
excludes by construction (`[flow.pr]`'s own default fast-lane platform);
`platform-build`/`platform-run` build the other five at release profile
via a `--profile release` toggle, same jobs the PR/main flows already
use at dev profile.

### The glibc floor: release/verify artifacts only, never the fast lane

`ci.toml [platforms.linux-x64/linux-arm64].wheel = "manylinux_2_17"` is
the fleet's declared floor for a **distributed** Linux wheel
(docs/policy-rust.md, proposal.md's target model) — every wheel that
`release-verify`/`publish` handle must meet it. It is enforced two ways:
`ci-lint release verify` (`PKG-006`) checks the staged wheel's filename
tag against it, and `ci/release.py glibc-check` independently parses the
staged wheel's bundled native CLI + PyO3 extension (`readelf -V`,
falling back to `objdump -T`) for the actual maximum `GLIBC_X.Y`
version-need string and fails if it exceeds the floor — proof from the
artifact's own bytes, not just the filename. Both `linux-x64` and
`linux-arm64` are built by `soldr wheel --release`, which links every
`*-linux-gnu` release wheel against soldr's catalogue glibc-2.17
toolchain, host target included (zackees/soldr#3432):
`linux-arm64` as `platform-build`'s cross target, and `linux-x64` as a
host-target build in `release-linux-x64`. The earlier
`cross-target: x86_64-unknown-linux-gnu` workaround is gone
(ghd#39).

**This floor is release-scope only.** The `fast` lane's PR-smoke wheel
(`ci/fast.py wheel-build`) stays a plain host build — untagged against
any manylinux floor, built for iteration speed, and never staged for
`release-verify`/`publish`. Only the artifacts those two jobs actually
handle (the release-profile wheel matrix) are held to glibc 2.17.

**The release wheel is built with `soldr wheel --release`; the sdist is
proven separately.** `ci/release.py build` writes the sdist through the
Soldr PEP 517 backend (`uv build --sdist`) and the staged wheel with
`soldr wheel --release [--target <triple>]` (zackees/ci.yml#17,
ghd#37). Since soldr 0.9.27
([zackees/soldr#3468](https://github.com/zackees/soldr/issues/3468)),
`soldr wheel` bundles `[tool.soldr.pep517] bundle-bins`, so the wheel
still carries `<dist>.data/scripts/ghd-cli` (PKG-003).
`ci/release.py sdist-smoke` then builds a wheel FROM the staged sdist
through the PEP 517 frontend and requires the bundled CLI in it (PKG-004).
That path needs soldr >= 0.9.26
([zackees/soldr#3451](https://github.com/zackees/soldr/pull/3451)), which
keeps `crates/ghd-cli` in the sdist's workspace `members`.

## What you CAN do locally today: build and inspect release artifacts

1. Confirm versions match in `pyproject.toml::project.version` and
   `Cargo.toml::workspace.package.version`.
2. `./ci.py all` passes locally.
3. Build the sdist and wheel through the real backend:
   ```
   uv build
   ```
   This drives the soldr PEP 517 backend (never a direct `maturin`
   call — see `ci.toml`'s `[allow] tools`), which builds the PyO3
   `_native` extension, builds `ghd-cli` via `soldr build --bin`
   under the same target/profile/cache environment, and stages both
   into one wheel with a regenerated `RECORD`. No `ci/build_wheel.py`,
   no post-build zip surgery.
4. Also build the wheel **from the sdist** (proves the sdist alone —
   not just the working tree — produces a working wheel):
   ```
   uv build --wheel dist/ghd-0.1.0.tar.gz  # use the actual sdist filename
   ```
5. Inspect the wheel with Python's `zipfile` module: confirm
   `*.data/scripts/ghd-cli` is present and its first bytes are an
   ELF header (`\x7fELF`) on Linux, that there is no `console_scripts`
   entry point, and that the wheel tag is `abi3` + the platform's
   manylinux/macOS/Windows tag as appropriate.
6. Install into a clean venv (not your dev `.venv` — see
   `ci.toml`'s cache/exceptions notes for where scratch venvs belong)
   and confirm `ghd-cli --version` runs, `shutil.which("ghd-cli")`
   resolves to that binary, and
   `ghd._native.__file__` ends in `.abi3.so` /
   `.pyd`.

## Cross-platform wheels

Every declared platform in `ci.toml`'s `[platforms]` builds through
soldr's Linux cross-compilation (`soldr cargo build --target <triple>`
/ `soldr wheel --target <triple>`) — no native macOS/Windows Rust
toolchain needed to produce the artifact, though native install-smoke
still needs the real OS (see `ci.toml`'s `[flow.release]`). This
replaces the old per-native-runner matrix build.

## Surface validation

Before tagging, run the two action gates to confirm the composite
action contract still holds:

```
./ci.py action_yaml
./ci.py action_surface
```

These are fast (<5 s) and catch the typo class of regressions where
`action.yml`'s scripts (`action/*.py`) drift from the binary's real
subcommand/flag surface.

## Tagging and GitHub Releases

Still not implemented — no git tag, no GitHub Release, and the `publish`
job **never uploads anything** (it mints and asserts an OIDC token, then
stops — "mock publish: stopping before upload"). The real sequence,
once the fleet is ready for an actual upload: version bump → tag →
`workflow_dispatch` on `main` with the tagged merge commit's exact SHA →
every lane green, `release-verify` green → `publish` prints claim PASS
for every assertion → (eventually) a real trusted-publish upload. Until
then, treat any release as a manual, out-of-band process and record what
you did in the PR/issue, not in this file.

## Dry-run readback and resume (zackees/ci.yml#8/#74, REL-003/004/005)

The mimalloc-pprof v1.0.1 exact-SHA release pilot
([zackees/ci.yml#8](https://github.com/zackees/ci.yml/issues/8)) hit
four latent publisher bugs, all on the real (non-dry) publish path. Two
of the lessons are now wired into `release-verify`:

- **REL-003 (readback)**: `release-verify` runs `ci/release.py
  mock-publish` (writes every staged artifact into a throwaway
  `${{ runner.temp }}/mock-registry` dir — standing in for the real
  publisher's destination) and then `ci/release.py mock-readback`
  (reads each artifact BACK from that registry, never from `dist/`
  directly, and writes `readback/<artifact>.json`:
  `{"path", "sha256", "read_back": true}`). `ci-lint release verify
  --readback readback` fails if any artifact's readback record is
  missing or its digest disagrees with the staged one — proof the dry
  run exercised the destination's *read* path, the exact class of bug
  mimalloc-pprof#562 was (`GET .../releases/tags/{tag}` 404s for a
  draft; only the paged list endpoint includes drafts).
- **REL-004 (resume)**: `workflow_dispatch` takes an optional
  `resume_from_run` input — a prior `release-verify` run id. When set,
  `release-verify` downloads that run's `release-dist-verified` artifact
  (which includes `release-manifest.json`) into `frozen/` and passes
  `--resume frozen/release-manifest.json` to `ci-lint release verify`.
  Every artifact path frozen in that manifest must still be staged with
  the IDENTICAL sha256 — this repo's release-profile builds are **not**
  byte-reproducible (mimalloc-pprof#564/#565), so a same-SHA re-dispatch
  that rebuilds is EXPECTED to fail this check; that failure is the
  point; it proves ci-lint refuses to (re)publish silently-rebuilt
  bytes. Actually reusing the frozen bytes on a real resume (rather than
  failing loudly and stopping) still requires restoring them from a
  retained `release-dist-verified`/`release-preflight-<sha>` artifact by
  hand or a follow-up automation change — out of this round's scope.
- **REL-005 (no-rebuild publish)**: `ci_lint.rules.release_gate.
  check_rel_005` (static) would flag a `ci/*release*publish*.py` script
  that rebuilds instead of downloading by run id. This repo's `publish`
  job already only ever downloads `release-dist-verified` (an
  `actions/download-artifact` step) and never calls a build tool itself,
  so it satisfies the rule's intent by construction — there is no
  standalone `ci/release_publish.py` in this repo for the static scanner
  to check, so REL-005 is proven at the design level here, not by a
  precheck finding.

## Perf suite

`[ci-perf]` (or `flow.release`/`flow.nightly`'s `suites = "all"`) runs
the `perf` job: `ci/perf.py bench` builds a release-profile wheel,
installs it into a clean venv, times `ghd-cli --version` startup
and one PyO3 call (`ghd.bindings.version_banner()`,
via `ci/perf_pyo3_bench.py` run by that venv's own python), and writes a
typed `BenchmarkFile` JSON. `ci/perf.py fetch-baseline` pulls the latest
successful `main`/nightly `perf-results` artifact through the REST API;
`ci-lint perf compare` reports every benchmark's delta either way and
only fails the job when `ci.toml [suites.perf].gating = true` **and** a
threshold is set (neither is true here — perf is informational).
`[ci-perf][no-test]` is the fast build+perf loop, never mergeable.
