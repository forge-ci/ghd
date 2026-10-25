# `ci/localrun/`

Implementation package behind the single `ci/local.py` entry point
(zackees/ci.yml#6 section 11, "bosn -> act: local CI is first class").
`ci/local.py` itself is only argument parsing and dispatch; every real
behavior lives here, split by concern the same way `ci/gates/` and
`ci/hooks/` are.

## Why this exists

zackees/zccache#1760: an agent sped up a bosn -> act loop by re-enabling
a cache family the fleet had retired, and reported the change as a win,
because **no local gate ran the guard that would have caught it**. The
fix has two halves, both here:

1. `ci/local.py precheck` must be fast enough, and unblocked enough
   (never rejected by `ci/hooks/tool_guard.py`, since it's a plain
   `python3 <script>` invocation), that an agent always runs it before
   reporting a workflow/cache/tool-shape change as done.
2. A local `act` run must be held to the **same** cache policy as
   remote CI, not a locally-convenient weaker one -- see
   `cache_audit.py`'s `ACT-001`.

## Modules

| Module | Runs where | Does |
|---|---|---|
| `ci_lint_checkout.py` | host | Clone/fetch `.ci-lint/` at `ci.toml`'s `linter` pin; no-ops (no network) once already warm at that SHA. |
| `precheck.py` | host | `uv run --no-project --with pyyaml python3 -m ci_lint precheck --repo . --local`, with `PYTHONPATH` pointed at `.ci-lint/`. Prints checkout + run wall time. |
| `event.py` | host | `ci_lint plan --act <path>` -- the one place act's `--eventpath` JSON is built, so a local run can't select a lane the real planner wouldn't. |
| `ci_toml_lite.py` | host + container | A narrow `tomllib` reader for just `[cache]` and `[local]` -- used where a full `ci_lint` checkout isn't guaranteed on `PYTHONPATH` (inside the container). |
| `sizes.py` | host + container | `parse_size("9GB") -> int`, duplicated from (not imported from) `ci_lint.rules.cache_static.parse_size` for the same reason. |
| `act_orchestrate.py` | host | `ci/local.py act`: cheap precheck, then shared gate execution/reuse and HEAD attestation; explicit selections are diagnostics. See the local-loop contract below. |
| `act_inner.py` | **inside** the bosn `act` stack container | `ci/local.py act-inner`: builds the clang-patched runner image if missing, runs `act` per lane against the host Docker engine (bind-mounted socket), labels and removes the sibling job containers it creates, runs the cache audit, writes `.act-local/result.json`. |
| `cache_audit.py` | container | `ACT-001`: audits act's local cache-server store against `ci.toml [cache].budget` and `[cache].retired`. |

## Current runner

The default requires a clean committed tree, delegates execution or fresh
result reuse to the pinned shared gate, and amends HEAD with attestation
trailers on success. The original complete workflow remains the execution
command, without a job filter. See the authoritative
[local-loop contract](../../CLAUDE.md#ci).
The legacy `act-inner` command refuses execution. The table entries for
`event.py`, `act_inner.py` and the Docker stack describe historical code,
not the current entry point. Cache audit remains a separate utility;
delegation alone does not prove the local store budget passed.


## act's cache-server on-disk format (empirically verified 2026-09-28)

`ci_lint`'s own `ACT-001` is still a stub (round 4 -- `ci_lint/runtime_stub.py`
lists it among the live-cache checks `--local` explicitly skips), and
zackees/ci.yml#6 open question 10 ("act's local cache store format") was
never resolved. This package is the first place it was. act's built-in
Actions-cache-server (`--cache-server-path`) is:

```
<path>/bolt.db        # a Go bbolt (embedded k/v store) index
<path>/cache/<xx>/<n> # raw payload blobs, opaque filenames
```

There is no pure-stdlib bbolt reader, and no documented schema. But
bbolt never compresses or encrypts a value, and this project's cache
server writes each index record as one literal JSON object
(`{"id":1,"key":"...","version":"...","cacheSize":N,"complete":bool,
"usedAt":N,"createdAt":N}`), which survives verbatim in the raw file
bytes. `cache_audit._read_index` recovers every record with a direct
regex scan (`_RECORD_RE`) plus `json.loads`, no bbolt library needed.
Verified by running a real `actions/cache/save@v4` step under act 0.2.88
and reading the result back with this exact regex.

Two independent gotchas this module handles:

- **bbolt is copy-on-write.** A stale pre-write copy of a record can
  still be present in the file bytes after an update. Records are
  deduplicated by `id`, preferring any sighting with `"complete":true`
  (an entry only ever transitions incomplete -> complete, never back).
- **The index's `cacheSize` field is not the audit's bytes-on-disk
  source.** The budget check sums real file sizes under `<path>/cache/`
  instead, so it can never be fooled by a stale or synthetic index
  record -- only the retired-family *key* check reads the index.

## Resolved: `dylint` lane green under act (zackees/ci.yml#47)

As of round M2-24, `python3 ci/local.py act --lanes dylint` passes end
to end -- see `ci/docker/act/README.md`'s "What a local run actually
exercises today" for the evidence table and the three defects fixed to
get there (a linked-git-worktree job-container mount gap in
`act_orchestrate.py`/`act_inner.py`, the `cache-budget` job missing its
`env.ACT != 'true'` skip in `.github/workflows/ci-pre.yml`, and a
shared-host anonymous GitHub-API rate-limit fragility in
`zackees/setup-soldr`'s release-tag resolution, which is an environment
condition, not a code defect here). The "GITHUB_TOKEN and cross-repo
checkouts" section below (the `.ci-lint` cross-repo checkout gap) was
resolved separately, in an earlier round, by the machine-scoped
`bosn.toml` `ci-lint` volume `ensure_ci_lint`/`act_inner.py` populate
directly instead of relying on `actions/checkout`'s `repository:`
override under act; the text is kept for its still-accurate root-cause
explanation and the `.secrets` opt-in it documents.

## GITHUB_TOKEN and cross-repo checkouts (known gap, with a working opt-in)

`ci.yml`'s `Checkout ci-lint (zackees/ci.yml, pinned)` step uses
`actions/checkout` with an explicit `repository:` override (a different
repo than the one being tested). Real GitHub Actions auto-populates
`github.token` for that; act does not, and `actions/checkout`'s `token`
input is `required: true` with no way to pass it as empty -- confirmed
by direct reproduction, both with no token (`Input required and not
supplied: token`) and with an obviously-fake placeholder value
(`authentication required: Invalid username or token`, i.e. GitHub
rejects invalid credentials outright even for a public repo's smart-HTTP
git clone -- it does not fall back to anonymous). This tool never
fetches, embeds, prints, or writes a credential anywhere (worker
contract: no secrets).

**The opt-in that already works, with no code or bosn changes:** `act`
defaults `--secret-file` to `.secrets` in its working directory, and
`_run_lane` never overrides that flag. `.secrets` (gitignored by this
repo, never created or read by this tool) sits at the bind-mounted repo
root -- `/work/.secrets` inside the container is the same file as
`.secrets` at this repo's root on the host. A developer who wants a
full local `act` run of `fast`/`dylint` adds one line,
`GITHUB_TOKEN=<their own PAT>`, to a `.secrets` file they create
themselves; `act` picks it up automatically on the next run. This is
act's documented mechanism, not a workaround this tool invented --
verify with `act --help | grep secret-file`.

## Wiring

- `bosn.toml`'s `[task.act-run]` (`cmd = "python3 /work/ci/local.py
  act-inner"`) is the only caller of `act_inner.py`.
- `.claude/settings.json`'s PostToolUse/Stop hooks call `ci/local.py
  precheck` through `ci/hooks/local_precheck_guard.py`.
