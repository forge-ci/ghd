# Update Procedure


Default `python3 ci/local.py act` requires a clean committed tree, uses the
shared gate's execution or fresh-result reuse, and amends HEAD on success.
See the [local-loop contract](CLAUDE.md#ci).
Explicit title/job selections remain diagnostics; native and release coverage
remain required.

When changing repo structure, CI gates, release flow, or agent
guidance, walk this checklist so nothing rots out of sync.

## Repo structure / CI

1. Update the relevant manifest, gate, or script. Any Rust/wheel
   command in a gate or script goes through `soldr` — never bare
   `cargo`/`maturin`.
2. If you added a gate:
   - `ci/gates/<name>.py` with `def run() -> int`.
   - Register in `ci.py::GATE_ORDER`.
   - Add a step to `.github/workflows/ci.yml` **once that file
     exists** — `zackees/ci.yml#6` round 1 deleted it; there is no
     workflow on this branch right now (see `docs/ARCHITECTURE.md`).
     Until a later round adds it back, the gate only needs to be
     reachable from `./ci.py <name>`.
   - Add a row to `tests/test_gates.py` covering the happy path.
   - If the gate needs a host check, import
     `ghd.platforms` — never `sys.platform`/`os.name`
     inline (see `src/ghd/platforms/README.md`).
3. If you added a hook:
   - `ci/hooks/<name>.py` reading JSON from stdin.
   - Wire in `.claude/settings.json` under the right event.
4. If you changed the Rust workspace shape (a new crate, a new ship
   feature set, a new platform target), update `ci.toml` in the same
   change — it's the checked-in contract a future `ci-lint` validates
   the repo against.
5. Update [README.md](./README.md) if the user-visible workflow changed.

## Architecture

6. Update [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) if crate or
   package responsibilities changed.

## Release flow

7. Update [docs/RELEASE.md](./docs/RELEASE.md) if build or publish
   behavior changed.
8. If the composite action's surface changed, update `action.yml` (and
   `action/cleanup/action.yml` if the cleanup contract changed), and
   re-run `./ci.py action_yaml action_surface` locally to confirm the
   gates still pass.

## Agent guidance

9. Update [CLAUDE.md](./CLAUDE.md) if agent-facing rules changed (the
   essential-rules list, hooks vs gates split, etc.).
10. Update [LINTING.md](./LINTING.md) if the lint surface changed.

## Versioning

11. For version bumps, keep Python (`pyproject.toml::project.version`)
    and Rust (`Cargo.toml::workspace.package.version`) aligned. The
    release pipeline assumes they match.

## Dropping the requirement

If a step on this list stops applying (e.g., gates aren't keyed by
GATE_ORDER anymore), update this file too. An update procedure that
references removed concepts is worse than no procedure.

## ghx parity: host and credential isolation (#7)

- Resolve the caller's host and credential before daemon discovery, including
  Enterprise token precedence and host-specific REST/GraphQL endpoints.
- Partition endpoint, startup lock, log, cache and in-flight work by a
  domain-separated credential/host digest, with an identity check on loopback.
- Support host-qualified repositories, GH_REPO and GraphQL --hostname; reject
  conflicting selectors and missing credentials before shared cache access.
- Preserve Python/CLI parity and pin resolved credentials when spawning.

## ghx parity: passthrough and mutation coherence (#8)

- Forward real-gh arguments and stdin/stdout/stderr, including rendering and
  stdin consumers, while preserving exit status and Unix signal conventions.
- Use a shared write journal before mutations/unknown commands; every daemon
  fences cached reads while writes are active or uncertain and checks the
  generation before returning a result. Both cache tiers are invalidated.
- Never retry a connected request after dispatch. Only absent/refused endpoints
  and explicit pre-routing authorization rejections permit rediscovery.
- Parse GraphQL documents and operationName instead of recognizing a mutation
  by a string prefix. Malformed and ambiguous operations fail at the boundary.
- Add GHD_GH_PATH and an absolute GHD_STATE_DIR override for explicit executable
  selection, private state isolation and deterministic tests.

## ghx parity: resource and collection reads (#9)

- Share PR/issue/run JSON across fields, projections and bounded list limits;
  isolate filters, run attempts, hosts and credentials. Nested reviews/checks
  and pending-checks exit 8 retain gh behavior.
- Retain collection bodies and ETags together for conditional Actions refresh.
  Rich workflow metadata and limits above 100 retain native gh fetching and
  pagination. A reached list limit is reported without claiming completeness.
- Cache exact output for other recognized headless read families. Unsupported
  and interactive forms keep the compatibility route; run watch is uncached.
- Expose the same reader as Python `gh_read`, with raw bytes and exit status;
  preserve nested JSON member order and delegate jq/templates to real gh on
  authenticated local HTTP with isolated renderer credentials/configuration.
- Enforce private directory custody through the existing kernal-api facade on
  all platforms; bound and validate endpoint/journal files before decoding.

## ghx parity: general REST GET bodies (#10)

- Retain safe relative-endpoint GET bodies and validators through the shared
  CLI/Python reader, preserving host/credential, headers and parameter scope.
- Reuse retained bytes after 304, replace bodies on changed ETags, use full GETs
  without validators and refuse no-store/wildcard-Vary retention. Range and
  diagnostic/paginated/input shapes retain native gh execution.
- Render jq/templates from raw retained bytes. Preserve native HTTP errors;
  never substitute an old authoritative body after upstream failure.
- Keep rate-limit introspection fresh and report HTTP operations/responses,
  revalidations and provider quota observations as separate measurements.

## Windows private-state correction

- Pair strict private-file reads with current-user directory ownership and
  explicit private-file creation, including elevated Windows tokens.
- Close the staged file before replacement and enforce private ownership on
  renderer configuration directories.
- Bound the lost-reply test server and expose child diagnostics promptly;
  retain its requirement that dispatched operations are never replayed.


## ghx parity: bounded retention and restart safety (#11)

- Share entry and byte limits across all read namespaces and their coalescing
  slots; reject invalid configuration and disable retention with either zero.
- Bound active reads separately, preserve identical-read coalescing and prevent
  evicted in-flight requests from recreating detached slots after completion.
- Add opt-in owner-private, identity- and generation-bound restart snapshots for
  reads with validators. Always revalidate restored bytes before reuse.
- Fence offline/overlapping/uncertain writes and stale publishers. Fail explicitly
  on corruption, incompatible schema, unsafe permissions or failed publication.
- Keep the short default TTL; add explicit per-command TTLs and optional longer
  completed-run projections restricted to stable fields and an explicit attempt.
  Mutable metadata retains its ordinary policy. Disk capacity is separate from
  the memory budget; origin no-store/no-cache/max-age still apply.


## ghx parity: shared budget and durable cooldown (#12)

- Govern daemon-managed operations with a shared four-slot identity pool,
  three routine slots and a configurable observed-budget reserve.
- Honor provider cooldowns, retry/reset hints and HTTP-200 GraphQL rate errors;
  distinguish authorization denial and preserve explicit authority refusal.
- Persist bounded private reservations/cooldown independently of response-cache
  settings, including canceled callers and overlapping operations.
- Attribute cache hits, dispatches, refusals and 304s to hashed caller metadata
  without changing body sharing; retain separate provider and estimate metrics.
- Run the complete installed cache-test directory on explicitly selected native
  integration lanes while preserving ordinary fast-lane coverage.

## ghx parity: repository snapshots and explicit freshness (#13)

- Add `ghd snapshot` and `ghd.bindings.snapshot` through one versioned protobuf
  operation and shared GraphQL selection, with bounded open PRs, default branch,
  reviews, changed files and check contexts. Preserve pagination evidence and
  expose aggregate truncation.
- Coalesce ordinary reads inside the shared retention/governance/coherence
  boundaries. Explicit fresh reads bypass the cache window and request origin
  revalidation; failures and marked stale responses return code 3 without JSON.
- Require explicit stale-planning opt-in, with source, age (nullable), reason
  and capture timestamp in schema-v1 output. Snapshot bodies remain memory-only.
- Verify the #12 fixture correction in hosted run 40506918999: the complete
  selected integration directory passes on Windows/macOS x64/ARM64, with CI OK.

## ghx parity: lifecycle, cache controls and configuration (#14)

- Add authenticated daemon start/stop/restart/status with lifetime ownership,
  serialized maintenance, active-work refusal and no PID signaling.
- Add redacted cache keys/stats and generation-fenced flush, including active
  publishers and persisted bodies. Governor and uncertain writes stay intact.
- Carry per-request bypass/TTL caps through CLI, Python and a versioned daemon
  route. Bypass neither reads nor publishes shared/durable bodies.
- Add bounded, typed owner-private JSON configuration with environment
  precedence, explicit invalid-input failures and per-user service guidance.

## ghx parity: bounded diagnostics, dashboard and replay (#15)

- Add a 256-row in-memory ledger and command summaries, with hashed callers,
  outcomes, latency, payload size, eviction/drop counts and explicit unknown
  dispatch attribution. Optional flag capture still redacts every value.
- Add authenticated read-only loopback ledger/HTML routes and cross-origin
  write rejection, without exposing credentials in dashboard URLs.
- Add bounded, network-free exact-key replay with input digest/provenance,
  explicit model assumptions and unknown provider-point savings.

## ghx parity: agent plugin and explicit managed gh (#16)

- Ship a local plugin manifest, scoped agent skill, generated Unix/CMD launchers and an
  explicit offline/HTTPS bootstrap using pinned version/member/SHA256 inputs.
- Reuse native wheel binaries and existing kernel hashing/private filesystem
  primitives for checksum/version validation and atomic tool publication.
- Add opt-in managed gh and own-shim rejection to the existing resolver, with
  explicit system-gh precedence and no download during ordinary calls.
- Document status, pinned upgrades, uninstall, bounded artifacts, custody and
  cleanup failures. Public release/catalog distribution is tracked separately in #20.
