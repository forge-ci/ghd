# Architecture


Default `python3 ci/local.py act` requires a clean committed tree, uses the
shared gate's execution or fresh-result reuse, and amends HEAD on success.
See the [local-loop contract](../CLAUDE.md#ci).
Explicit title/job selections remain diagnostics; native and release coverage
remain required.

## Goal

Ship one Python wheel that exposes:

- a Python import surface backed by `PyO3` (`ghd._native`)
- a command surface backed by a compiled Rust executable (`ghd-cli`,
  bundled into the wheel by the soldr PEP 517 backend at
  `ghd-<ver>.data/scripts/` — pip extracts it
  straight into the venv's `Scripts/`/`bin/` on install, no Python
  launcher; see #7 for the Windows `os.execv` race that motivated this)

A consumer installs a single distribution and gets both deliverables.
A maintainer maintains one workspace and (eventually, once a later
round of [zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6)
lands it) one generated CI matrix to feed both.

## CI Architecture

This template implements the canonical Rust+Python gate/hook shape
from [`zackees/zccache#835`](https://github.com/zackees/zccache/issues/835),
and is the fleet reference repo for the `rust-pypi-app` profile defined
in [`zackees/ci.yml#6`](https://github.com/zackees/ci.yml/issues/6).
Load-bearing pieces:

1. **`ci.toml`** (repo root). The CI contract: declared platforms, the
   Rust public/private crate split and test-binary budget, Python
   packaging shape, suites, flows, tags, cache families. Does not
   generate YAML — a future `ci-lint` (built in `zackees/ci.yml`)
   checks the repo against it and computes each run's plan.
2. **`./ci.py`.** PEP 723 dispatcher, directly executable via its
   `#!/usr/bin/env -S uv run --no-project --script` shebang. Every
   gate invocation goes through here so the `--no-project --script`
   flag combo (which suppresses the soldr-backend auto-build trap)
   lives in one place.
3. **`ci/gates/`.** Workspace-state checks. Each file exposes
   `def run() -> int`. Canonical ordering in `ci.py::GATE_ORDER`. Every
   Rust/wheel command goes through `soldr` — never bare `cargo`/`maturin`.
4. **`ci/hooks/`.** Agent-intent guards wired through
   `.claude/settings.json`. Only fires during Claude/Codex sessions.
5. **No `.github/workflows/` right now.** `zackees/ci.yml#6` round 1
   (this change) deleted the previous `ci.yml` — it queued a retired
   `macos-13` runner for 24h on every run and never passed. A later
   round adds `.github/workflows/ci.yml` (the only trigger-bearing
   workflow) + `ci-pre.yml` (workflow_call only), planned by
   `ci.toml`. Until then, `./ci.py all` locally is the equivalent.
6. **`action.yml` + `action/cleanup/action.yml`.** Composite action
   contract. Validated by `ci/gates/action_yaml.py` (structural) +
   `ci/gates/action_surface.py` (runtime binary surface match). Every
   `run:` step is one line calling a script under `action/` — no
   inline multi-line shell.

## Crate Responsibilities

### `crates/template` (public amalgam)

- **No logic of its own.** Every symbol is a `pub use` re-export from
  a crate under `crates/private/`.
- The only crate published to crates.io, and the only one
  `ghd-cli`/`ghd-py` depend on.
- Cargo features are `dep:` wiring only (`json = ["dep:ghd-daemon"]`)
  — never a behavioral `#[cfg(feature = ...)]` branch.

### `crates/private/ghd-core`

- pure Rust domain logic, `publish = false`
- no Python-specific concerns, no `[features]`
- reaches the OS only via `crate::platform::*` (aliased from
  `ghd-platform`)
- the unit of behavior consistency between the CLI and Python surfaces

### `crates/private/ghd-daemon`

- an optional capability (JSON encoding of the version banner) that is
  a whole crate, not a `#[cfg(feature)]` branch inside `ghd-core`
- no third-party dependencies — hand-rolled, deliberately

### `crates/private/ghd-platform`

- the host-platform facade: one `cfg_select!` selector (`lib.rs`), a
  cfg-free facility surface (`platform.rs` + `platform/**`), native
  code confined to `platforms/{windows,linux,macos}/**`
- the **only** crate allowed to select on host OS; every other crate
  reaches it through `pub(crate) use ghd_platform as platform;`

### `crates/ghd-cli`

- argument parsing (currently minimal), command execution
- stdout/stderr policy, exit-code policy
- depends on `template` (the amalgam) with the `json` ship feature —
  never on the private crates directly
- **subcommand/flag surface is part of the composite-action contract**
  — `ci/gates/action_surface.py` verifies `action.yml`'s scripts only
  reference surface actually present in `--help`

### `crates/ghd-py`

- `PyO3` module definitions (`#[pymodule]`, `#[pyfunction]`)
- Python-friendly value conversion
- thin translation layer over `template` (the amalgam) — no Rust test
  harness (`[lib] test = false, doctest = false`); tested from the
  Python side (`tests/test_bindings.py`)

### `src/ghd`

- package version and re-exports (`__init__.py`)
- Python wrapper around the extension (`bindings.py`), no pure-Python
  fallback for `_native`
- `platforms/` — host-platform facade (Python side), mirroring the
  Rust crate of the same purpose
- typing stub for the PyO3 surface (`_native.pyi`)

The `ghd-cli` binary is intentionally NOT under the package — it
ships via `bundle-bins` (see Goal above). A Python caller that wants to
invoke the CLI from code should do
`subprocess.run([shutil.which("ghd-cli"), ...])`, not import
anything from this package.

## Hook / Gate Split

| Concern                                | Home                       |
|----------------------------------------|----------------------------|
| Repo-state checks (every CI cycle)     | `ci/gates/*.py`            |
| Agent-intent checks (Claude sessions)  | `ci/hooks/*.py`            |
| File size budget (workspace-wide)      | `ci/gates/loc.py`          |
| File size budget (per-edit)            | `ci/hooks/loc_guard.py`    |
| README presence + size                 | `ci/hooks/readme_guard.py` |
| Bare cargo/maturin / unsafe `uv run` shape | `ci/hooks/tool_guard.py` |

## Non-goals

- duplicating core logic in Python
- embedding CLI-only behavior inside the `PyO3` module without an API
  use case
- maintaining separate CI infrastructure for the Python and Rust sides
- inline shell logic in any future workflow YAML, or in `action.yml`
- gates that hard-couple to the agent's session (those are hooks)
- a `#[cfg(feature = ...)]` branch inside a private crate's shared
  logic — features gate `dep:` wiring in `template` only

## Credential and host boundary

`ghd-core::identity` resolves the hostname and token in the caller. Daemon
endpoint discovery uses `identities-v1/<digest>` under the private state
root. The digest includes a versioned domain separator, normalized hostname
and credential. It contains no raw credential. The spawn environment pins
the resolved identity, preventing a stored-account change between discovery
and daemon startup from changing the cache owner.

A daemon serves one identity. Its cache and batch maps are private to that
process, and every loopback call carries the expected identity digest as
well as the session bearer token. Requests for another identity are rejected
before cache access. REST and GraphQL URLs derive from the selected deployment
(public GitHub, ghe.com or Enterprise Server). Both native Python clients and
CLI callers use the same core discovery/resolution path. Legacy unpartitioned
endpoint files are never reused.

## Write coherence and passthrough

`ghd-core::compat` launches real gh without a shell, inheriting the original
streams and arguments. Local auth/config commands and known reads are separate
from mutations and unknown commands. A write intent precedes subprocess
execution. A successful exit finishes it; failed/uncertain execution retains
it, while a definite spawn failure can safely finish without remote dispatch.
Exit codes propagate through a typed command-status error and the native CLI.

`ghd-core::coherence::Journal` stores a versioned protobuf generation and
outstanding write IDs under the private state root. File locks serialize
journal transitions across processes; file replacement publishes each record
atomically. The journal is shared across identities because an arbitrary gh
alias/extension can select a different host or credential than a superficial
argument scan would infer. It contains no credentials, repository names or
mutation payloads. Corrupt or unresolved records fail closed.

Daemon reads check the journal, synchronize their cache generation and check
again before returning bytes. A write clears PR and raw-query maps, and a
pre-write in-flight result cannot become authoritative after the generation
changes. The same generation travels with optional persistent records. A
snapshot captured before a stopped-daemon mutation cannot be relabeled with a
new epoch. Restored bytes always require origin validation; this contract does
not depend on treating directory publication as a power-loss transaction.

GraphQL classification uses an actual parser in ghd-core; operationName is
part of both the upstream envelope and the exact query cache key. The daemon
rejects invalid/ambiguous documents and subscriptions before execution.
Client reconnection is restricted to pre-dispatch situations; once connected,
a lost reply is an error rather than a second potentially mutating request.

## Shared resource reads

The core `resource` parser binds explicitly scoped read targets and normalizes
JSON projection, filters and bounded limits. `reads` sends the caller's real-gh
program stamp, output context and worktree-dependent context over protobuf to
`/v1/read/gh`. The daemon verifies host/identity, program and argument bounds.
CLI and Python use this one path; existing scalar PR GraphQL batching stays
intact. Unknown fields retain exact gh error output rather than guessed data.

The resource cache has a lock per normalized request. Canonical PR/issue/run
JSON retains a union of fetched fields; projections preserve nested member
order and nulls. Each list can share 100 rows across smaller requested limits.
Larger limits use native gh pagination and an exact-output key. The Actions
collection adapter maps only fields available in the REST collection, keeping
its ETag with the body. Conditional requests execute `gh api --include` using
the same caller-bound configuration, proxies and CA context as other resource
reads; they never borrow the startup HTTP client's transport. A 304 without a
retained validated body is an error.
`workflowName` and jobs retain native metadata fetching, since the collection
validator cannot establish those separate dependencies' freshness.

Other allowlisted read families retain exact stdout/stderr/status. Real-gh
subprocesses have bounded output, a deadline, four shared permits, explicit
caller context and identity-pinned credentials. This includes uppercase and
lowercase proxy variables, PATH and cache/configuration directories. Debug or
forced-terminal requests bypass cached CLI execution and are rejected by the
read-only binding. They start with a cleared
environment rather than inheriting the first daemon caller's output context.
Every cache read checks the shared write epoch before and after publication;
mutations invalidate PR, raw GraphQL and resource maps together.

The renderer serves the projected body on authenticated loopback HTTP and
executes real gh's jq/template engine against that endpoint. Its random
credential stays in the child environment, its configuration directory is
private, external proxies fail closed and only the local GET route is served.
The directory owner remains alive until the bounded child is reaped and then
cleans it. Render failures retain native stderr/status and do not overwrite
the canonical cached JSON. This feature adds no hosted workflow.

## General REST bodies

Core `rest.rs` classifies safe GET request shapes and normalizes the explicit
host/method, while retaining every representation header and parameter in the
request identity. Unsupported diagnostics, page aggregation, caller validators,
Range/auth headers and file/stdin inputs keep real gh's original execution.
Unsafe methods continue through the shared write-intent compatibility route.

`rest_reads.rs` stores raw bytes with their ETag, stderr, validation time and
bounded freshness policy. Per-key locks coalesce concurrent refreshes. Every
fetch uses bounded, credential-pinned gh execution with the caller's transport
context. No-store/wildcard-Vary responses cannot persist in the map, and
no-cache/max-age policies constrain the short application freshness window.
A 304 requires the matching retained validator/body; a 200 replaces them and a
missing validator causes the next stale refresh to fetch the full body.
The same generation fence clears the shared retention store and rejects old
in-flight publications.

Transformations run through the private local gh renderer on every read;
canonical body sharing does not freeze time-dependent template output. Error
bodies preserve native status/stderr without becoming authoritative entries.
Quota telemetry stores bounded provider bucket observations separately from
logical HTTP dispatches and validated 304 reuse. It does not attribute shared
provider totals to individual commands or claim to count internal redirect
hops. `rate_limit` itself bypasses body reuse and records fresh observations.

Windows state custody uses the owner-private directory API and private-file
creation together. Both directory and file owners must be TokenUser, including
under elevated tokens whose default owner may be Administrators. The reader
continues to reject mismatched owners and ACLs; general filesystem directory
privacy alone does not establish the stricter state-file contract.


## Shared retention and conditional restart bodies

`ghd-core::retention::Cache` owns an entry/byte-bounded LRU index. The daemon
stores all four typed slot namespaces in that one index, using domain-separated
digest keys. Empty coalescing slots receive a bookkeeping charge too. Resource
response bytes, selected-field storage, ETags and decoded PR fields contribute
to the charge. The decoded PR tree uses a conservative serialized-size multiplier;
these are retention accounting limits, not an allocator or RSS promise.

A slot's Arc identity is its publication lease. Completion reserves the new
weight only if that same slot remains indexed. Eviction can detach an active
slot but cannot let its eventual fetch recreate an evicted entry. A 32-permit
read admission pool bounds detached active owners; the separate four-permit
upstream child pool remains owned until each bounded child is reaped. Identical
admitted reads still coalesce. Busy admission fails after one second.

`ghd-core::durable::Store` provides the optional restart tier. It serializes
bounded records in one private snapshot with schema, identity, original read
epoch and BLAKE3 checksum. A stable private lock serializes read/modify/replace
across processes; lock acquisition is bounded. The store verifies the journal
before loading, before publication and after publication. Changed epochs drop
old records rather than rewriting their epoch. Crashed or overlapping writes
remain fenced through the existing write-intent journal.

Only validator-bearing PR, run-collection and REST records are saved. Restored
entries start stale; origin 304 permits matching-body reuse, while a changed
200 replaces bytes. A restart never turns a saved timestamp into freshness.
No-validator/exact CLI and raw GraphQL reads fetch normally after restart.
No-store or missing validators remove previously persisted REST records.

Memory and disk each have the configured entry/byte caps; neither is an RSS
limit. Snapshot parsing/encoding and admitted responses require bounded working
buffers outside retained-body accounting. Snapshot corruption, identity/schema
mismatch, excessive size, unsafe native permissions and I/O failures are
explicit errors. Under the exclusive lock, leftover publication stages require
explicit recovery before further writes, preventing repeated crashes from
accumulating more staged cache bodies. They are never silently deleted. The
existing private-file primitives work across supported
hosts. Atomic replacement and file sync cover process interruption; directory
metadata and power-loss durability are not claimed. Origin validation remains
mandatory after every restart. No five-minute minimum or implicit immutable
lifetime is introduced, and run attempts remain part of their resource keys.


`cache_policy::Policy` validates explicit per-command millisecond overrides and
an optional completed-run TTL. That extension applies only to completed `run
view` reads with an explicit attempt and a stable-field allowlist. Freshness is
computed for each projection, so a later mutable-metadata request cannot inherit
the longer lifetime of a stable projection sharing its cached body. Observed
origin no-store/no-cache/max-age policy caps both ordinary and configured lifetimes,
including run collections and PR sentinels. Opaque native gh responses expose
no headers; their application TTL comes from the explicit command policy. Restored validation timestamps are
explicitly absent instead of being reconstructed from a wall-clock timestamp.

A response under validation or rendering lives in its admitted request, with
the indexed slot empty. Only successful reservation and publication attach it
back to that slot. Cancellation and failed revalidation therefore cannot leave
an undercharged body indexed. Detached GraphQL batch workers own separate
32-permit capacity until completion, and each batch admits at most 32 distinct
PRs. Subscriber cancellation cannot free a worker's still-live capacity.


## Shared dispatch budget and durable cooldown

`ghd-core::governor::State` evaluates provider buckets and throttle deadlines
against an explicit clock. REST core/search/code-search categories and GraphQL
remain distinct. A cooldown applies across that credential identity. Plain 403
permission denial is not a rate signal; 429, depleted/retry-marked 403 and typed
or recognized GraphQL rate errors are. Retry-After seconds/dates and primary
reset hints control the deadline; missing hints use bounded exponential delay.

`governor_ledger::Ledger` serializes admission/completion with the existing
bounded private-directory custody helper. Its 16 KiB versioned record binds a
checksum to the record type and identity; it contains at most four pending
reservations and sixteen provider buckets. Missing/corrupt custody never resets cooldown silently.
Three nonessential operations may be pending; the fourth slot is protected.
Each reservation records its category/window and minimum estimated units.
A 304 refunds only its own same-window reservation. Out-of-order observations
cannot roll back a newer window. Successful introspection can observe other
provider resource buckets without canceling an active secondary cooldown.

The daemon mirrors those limits with owned semaphores. Upstream HTTP requests
have a 60-second total deadline; gh children have bounded output and a 60-second
execution deadline. A canceled HTTP operation leaves its durable reservation;
a blocking child's owner carries its process/governor permits until reaped.
Expired unknown reservations cause a conservative sixty-second cooldown from
their 180-second lease deadline. Explicit cancellation before known dispatch
can finish its reservation. Actual dispatch is never replayed automatically.

Essential admission precedes a GraphQL mutation's write intent, so a refused
mutation does not leave a false uncertain-write fence. Accepted writes still
record intent before actual upstream dispatch, invalidate retained generations,
and retain intent when completion is uncertain. Explicit no-cache REST reads
and fresh quota introspection can use the protected slot; only introspection
bypasses quota/cooldown refusal. Fresh cache hits need no governed dispatch.
There is no hidden stale fallback and no automatic upstream retry loop.

A task-local caller digest is carried separately from protobuf body keys and
explicitly propagated to detached batch workers. Batches attribute their real
dispatch to their first request; waiting callers retain their request/cache-hit
counts. Metrics have a bounded caller table with an overflow bucket, reset at
daemon startup and expose no raw labels, cwd or credentials. Provider headers,
logical HTTP/gh dispatches and minimum-unit estimates are distinct quantities.
Internal redirects, gh subrequests and external users are not measured points.

This policy covers daemon-managed read/HTTP routes, including its direct
GraphQL mutation route. Native passthrough/interactive streams retain their
original behavior and are not intercepted. It does not claim a machine-wide
proxy, per-caller quotas or a priority scheduler. Selected native integration
commands run the whole installed-test directory; unmarked controlled-cache
tests also keep their existing routine fast-lane coverage.

## Repository snapshot protocol

`ghd-core::snapshot` owns flag parsing, the read-only GraphQL selection,
repository validation and JSON schema-v1 envelope. CLI and PyO3 send
`SnapshotRequest` version 1 to `/v2/snapshot`; an old daemon cannot silently
ignore authority flags on an older generic read route. Repository and identity
hosts must agree. Backend environment/program/caller context uses the existing
read binding and cache-key helpers.

The daemon keeps a fifth retained slot type for snapshots. A slot serializes
ordinary callers, including callers arriving during the initial fetch, and
charges its body to the existing global LRU. It retains the original epoch
and checks the coherence journal before returning either fresh or stale data.
Fresh requests bypass every retained body, take essential governor admission
and invoke real gh with HTTP no-cache headers. No snapshot uses the generic
GraphQL cache or optional durable body store. Origin no-store forbids retention;
origin no-cache/max-age constrain ordinary reuse.

The document has `schema: 1`, `repository` (raw selected GraphQL fields),
`truncated` and `freshness`. Freshness includes `source` (fetched, shared, fresh,
stale), `stale`, nullable `age_ms`, receipt `captured_at_ms` and `reason`.
Repository connections preserve pagination/cursors/counts. Any unselected page
or count beyond selected nodes marks the document truncated. GraphQL partial
errors, missing pagination, wrong repository and mismatched PR/check commit
identity refuse the result. A null default branch and absent check rollup are
valid. No automatic pagination or completeness assertion is made.

Positive upstream Age, Warning 110 and stale gate headers prevent an authority
result. An unknown downstream age stays null. Without explicit planning opt-in,
an expired body plus upstream failure returns code 3 with no JSON. Opt-in can
return the old body with increasing age and a bounded reason, but cannot cross
a mutation epoch. Fresh and stale-planning options are mutually exclusive.
Transport/configuration errors remain distinct from service freshness refusal.

## Operational ownership and request policy

Core operations reuse the private-state directory lock helper for separate
maintenance and lifetime-owner directories. A foreground daemon acquires the
owner lock before binding/publishing an endpoint; detached autostart retains
the existing start lock and health check. Maintenance serializes explicit
starts/restarts. Stop uses the current authenticated endpoint and never a PID.
The daemon closes admission atomically with checking active handler count and
upstream/read permits. Busy stop returns 409. Accepted stop schedules runtime
cancellation after a short response grace, removes only its own endpoint and
releases ownership on exit. The client waits boundedly for that release.
Dropped handlers are counted through a guard; detached upstream workers retain
their existing permits until reaped.

Flush advances the global journal through a completed intent, then clears
local retained slots. Old requests cannot publish/return against that epoch;
other identities and durable bodies also observe the new generation. The
operation does not remove outstanding intents or reset admission cooldown.
Inspection emits namespace/digest keys, never raw query/credential/body data.

CLI/PyO3 request policy uses a scoped thread-local value while constructing
the synchronous transport call. Controlled reads use v2 route names (v3 for the already-v2 snapshot) and a
strict versioned policy header, so old daemons fail instead of ignoring flags.
The daemon holds policy in an async task-local separate from cache keys and
identity. Bypass creates an unindexed slot and suppresses durable restoration
and saving; ordinary callers retain their original cache. TTL is a final cap
after command/settled policy and cannot extend origin policy. HTTP API bypass
fetches add no-cache. Snapshot authority remains a separate explicit contract.

Configuration reads use bounded native owner-private file validation and the
existing JSON parser. Only known numeric settings and the typed command-TTL
object are allowed; credentials and paths are not configurable there. The
environment wins over a valid file, then defaults apply. Explicit lifecycle
changes validate configuration before stopping, and a new daemon reads its
configuration before endpoint publication. No environment mutation or second
platform selector was introduced.
