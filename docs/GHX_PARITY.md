# ghx parity implementation

This document records the implementation of the native children of
[meta issue #6](https://github.com/forge-ci/ghd/issues/6).
Changes are implemented sequentially in the ordinary repository checkout.
No Git worktrees are used.

## Completion rules

A capability is complete only after its acceptance criteria have evidence.
A design, passthrough alone, a stub, or a passing unrelated test does not
establish parity. Each capability has a separate implementation commit.
Implementation PRs and the standing upstream tracker link the resulting code.
The recurring review issue stays open as a maintenance responsibility.

## Work order

| Issue | Capability | State |
|---|---|---|
| #7 | Host and credential isolation | Implemented and verified in 646f5db; draft PR #18 |
| #8 | gh passthrough and mutation coherence | Implemented and verified; draft PR #18 |
| #9 | Expanded resource and collection reads | Implemented and verified; draft PR #18 |
| #10 | General REST cache and revalidation | Implemented and locally verified; draft PR #18 |
| #11 | Bounded durable cache and restart invalidation | Implemented and locally verified; draft PR #18 |
| #12 | Shared rate governor and cooldown | Implemented and locally verified; draft PR #18 |
| #13 | Repository snapshots and authority freshness | Implemented and locally verified; draft PR #18 |
| #14 | Daemon/cache operations and configuration | Implemented and locally verified; draft PR #18 |
| #15 | Diagnostics, dashboard and replay | Implemented and locally verified; draft PR #18 |
| #16 | Agent plugin and managed gh distribution | Pending |
| #17 | Standing upstream inspection list | Existing assigned tracker; final reconciliation pending |

## Host and credential isolation (#7)

The regression `credentials_and_hosts_cannot_share_a_daemon_directory`
was added before the implementation. The original `./ci.py test` failed
because two tokens both selected the unpartitioned per-user `ghd` directory.
The implementation resolves the host and credential before endpoint access,
partitions daemon state by a domain-separated cryptographic digest, pins the
resolved credential in the child environment and checks the expected identity
on every loopback call before routing to a cache.

The core resolver tests cover credential precedence, fallback binding,
missing credentials, token rotation, host validation, repository remote forms
and deployment-specific API URLs. Parser tests cover repository/hostname
conflicts. The Python extension calls the same repository parser as the CLI.
The loopback test concurrently reads seeded PR and raw GraphQL caches under
two different credentials and another host; it also checks that an incorrect
identity is rejected despite possession of the fixture session bearer token.
These tests use fixture credentials and do not contact GitHub.

Validation: `./ci.py all` passed loc, fmt, clippy, ruff, build,
backend_smoke, action_yaml and action_surface. Its initial pylint/test
failures (the Python signature stub and a test whitespace assumption) were
fixed, then `./ci.py pylint` and `./ci.py test` passed, including 63 installed
Python tests. The focused core directory regression also passed in review.

The review found a test-isolation risk from inherited freshness settings.
Fixtures now use a per-state non-expiring policy and a disabled upstream URL.
`GHD_FRESH_MS=0 soldr cargo test -p ghd-daemon --lib
server::identity_tests::concurrent_credentials_cannot_share_loopback_cached_answers`
passed with the strengthened fixture. Production freshness behavior is unchanged.
The contract precheck returned zero violations (its existing local-only
needs-review items remain explicitly reported by that tool).

## Remaining work

The pending rows above retain the full scope of the meta issue. Implementing
identity isolation does not complete the parity goal. In particular, broad
read support, persistence coherence, rate control and distribution need their
own focused regression and implementation evidence.

Before delivery, fetch and rebase onto `origin/main`, resolve conflicts, run
the required checks against the rebased tree, reconcile GitHub issue states
and verify a clean checkout with no temporary artifacts left behind.

## Passthrough and mutation coherence (#8)

RED: a legal leading fragment hid a later mutation from the previous prefix
classifier. The focused daemon regression failed before replacing that code.
The implementation uses graphql-parser 0.4.1 with explicit operation selection.
The shared write journal fences both cache tiers across subprocess writes,
concurrent reads, uncertain outcomes and daemon restarts. The transport no
longer retries a connected request when its reply is lost.

Initial targeted core, daemon and CLI tests passed, including the controlled
write/read overlap and uncertain-write restart schedule. The passthrough
fixture verifies literal arguments, stdin, stderr and exit 7 without real gh
or GitHub. The fixture's failed write leaves a journal fence, as required.
The lost-reply and stdin/envelope regressions pass. `./ci.py all` completed
with ALL GATES PASSED, including installed-wheel Python tests and backend smoke.
The final CLI suite passes all seven tests, including explicit auth/config
passthrough without credentials or state creation and the updated help surface.
The read-only review is clean and its focused mutation regression passes.
Contract precheck reports zero violations with its existing local-only
needs-review items retained.

One build restored stale platform metadata that omitted a function present in
source. Only local ghd-platform build artifacts were moved to trash, then
`ZCCACHE_DISABLE=1 ./ci.py build` succeeded. The following normal-cache full
gate run also passed; no gate was relaxed to work around this observation.
The journal's process-crash fence is implemented here; power-loss validation
and durable cache custody remain part of #11 rather than being claimed done.

## Resource and collection reads (#9)

The installed-CLI regression `test_issue_superset_and_subset_share_one_cold_fetch`
originally returned correct fixture JSON but failed because the two equivalent
issue reads executed the upstream executable twice. Its temporary daemon is terminated and
reaped by the fixture. This is the RED signal for the new resource cache.

A controlled probe confirmed that real gh can render both `--jq` and templates
from an authenticated loopback HTTP response using an absolute local endpoint
and `--hostname 127.0.0.1`. Real credentials/configuration were replaced and
external proxy access was blocked. The nonce can therefore stay in the child
environment rather than a process-list-visible header argument. The reader now
uses that renderer to preserve gh semantics without adopting a second jq.

Current gh source and installed help were checked for run-list behavior.
The default limit is 20. `workflowName` is resolved separately from a run's
REST `name` field; do not copy the upstream fork's direct name-to-workflowName
mapping. Conditional collection reuse must be limited to fields/dependencies
whose freshness is actually covered, while preserving the native metadata
lookup for other shapes. Large limits/pagination and attempts must remain
explicit; no truncated collection will be presented as complete.

## Read command matrix

All rows below have a daemon cache implementation for the listed headless
shapes. "Exact" means retaining real gh stdout/stderr/status under a full
argument/context key, rather than projecting a canonical resource. This is
cached execution, not an uncached passthrough or a claim that every gh flag is
implemented. Unknown, interactive, web, streaming/watch, URL-based cross-host
and unbound repository shapes keep real gh execution. The standing #17 review
tracks future upstream shape/schema changes; generic REST is the separate #10.

| Family | Cache behavior in #9 |
|---|---|
| PR view/list/checks | Canonical numeric JSON resources, nested fields and projections; checks exit 8/1; scalar direct views retain batching |
| PR status/diff | Exact output with explicit repository |
| Issue view/list | Canonical numeric JSON resources and projections |
| Issue status | Exact output with explicit repository |
| Run view/list | Canonical numeric JSON resources/attempts; collection ETags for collection-derived fields; native metadata for workflowName/jobs |
| Repo view/list | Exact output; repository view selector is explicit |
| Workflow list/view | Exact output, including view --yaml, explicit repository |
| Release list/view | Exact output with explicit repository |
| Search repos/issues/prs/commits/code | Exact output, argument/filter context retained |
| Label list | Exact output with explicit repository |
| Gist list/view | Exact output for identifiers; cross-host URL forms stay native |
| Project list/view/field-list/item-list | Exact output, owner/filter arguments retained |
| Cache list | Exact output with explicit repository |
| Ruleset list/view/check | Exact output with explicit repository |
| Org list | Exact output |
| Secret list; variable list/get | Exact output with explicit repository or organization |
| API GraphQL | Existing native query cache/write fence; generic REST follows #10 |

Regression coverage includes a cold-call-count case for every exact-output
row, shared projections and filtered list limits, attempts, nested/null data,
unsupported field failures, real gh jq/template rendering and cleanup. A local
mock GitHub transport compares the installed real gh with cached CLI and
Python output for the main PR/issue/run JSON shapes, including pending checks.
The real-gh transport comparison runs on Unix hosts with gh installed; the
synthetic cache tests are portable across declared platforms; native Python
integration execution is selected explicitly with ci-test-integration. No live GitHub answers
or performance claims are used as test evidence.

The first implementation passes the issue-projection RED regression and the
expanded synthetic resource tests. The real-gh comparison exposed nested
review member-order changes; projection now uses kernal-api's bounded
member-preserving parser. The collection tests retain the correct rows and reject an unsolicited 304.
The installed real-gh socket fixture additionally exercises a real local
200/ETag then 304 schedule, with no startup-transport substitution. That
transport regression failed against the previous installed wheel and now
passes. The subprocess-capacity regression proves that cancellation does not
release a permit before the child is reaped. Review also tightened positional
URL isolation and diagnostic/proxy contexts.

Real gh 2.98 reports pending JSON checks with exit 0; its plain checks output
returns 8. Tests compare both forms to the real binary and preserve their
actual statuses. A synthetic JSON8 fixture additionally covers backend
versions returning 8 for that shape. Mock responses include schema introspection
and private per-test gh caches rather than relying on ambient HTTP receipts.
Validation: the final `./ci.py all` reports ALL GATES PASSED, including all
Rust suites, 111 installed Python tests, backend wheel smoke and action gates.
The 48 resource tests cover every exact-output command family in the matrix,
real-gh CLI/Python goldens, 101-row pagination, diagnostic bypass and retained
ETag bodies. Read-only clud-review is clean after the reported corrections.
Contract precheck reports zero violations; its 17 existing local-only
needs-review checks remain visible. No gate was weakened or workflow added.

## General REST cache (#10)

The installed `test_rest_get_caches_the_response_body` first failed because two
identical GETs both reached the private local API. Its response bytes matched;
the cold-request count exposed the missing retention. The shared installed
API test harness was extracted separately in bde3b41, with all 111 tests and
Python lint passing, before adding REST behavior.

The parser accepts relative, explicitly host-bound safe GET shapes and keeps
methods, parameters, representations and caller context in the cache identity.
The daemon retains bodies and ETags together, renders jq/templates locally and
checks the write epoch around publication. Conditional calls use the same
caller-bound real-gh transport as resource reads. No-store responses and
wildcard Vary cannot enter retained data. Missing validators use full GETs;
rate_limit introspection always goes upstream.

Explicit response-header display, pagination/slurp, verbose/silent output,
caller validators, Range, alternate Authorization/Cookie and file/stdin inputs
retain real gh execution. These shapes are not claimed as conditional-cache
support. Native pagination still preserves every page. The published raw
GraphQL route and batching stay separate and unchanged.

HTTP operations and final parsed responses are counted separately from 304
revalidations. Provider-reported budget observations retain their resource,
used/remaining/limit/reset fields. These are shared provider totals, not an
invented per-command point cost or a count of redirect hops. Verification results are recorded below.

The first installed REST gate passes all 121 Python tests and the Rust suites.
Review then added caller If-Match/If-Unmodified-Since bypass and conservative
request max-age validation. Bounded native status conversion now lives in the
existing platform adapters: Unix signals become shell codes and Windows crash
status bits remain intact. These corrections are included in final validation.

Final local validation: `./ci.py all` reports ALL GATES PASSED, including
122 installed Python tests, core41/daemon19/platform5 Rust tests, backend wheel
smoke and action checks. Read-only clud-review is clean after the request-policy
correction. The REST suite includes a portable synthetic subprocess case and
real-gh Unix API cases; the platform adapter has native Windows crash-bit tests.

## Native Windows validation follow-up

[Hosted run40473014155](https://github.com/forge-ci/ghd/actions/runs/40473014155)
for adc690e passes fast, Dylint and native macOS x64/arm64. Windows x64/arm64
cross-builds pass; native tests report a write-journal failure and hang in the
lost-reply fixture until job cancellation. That run was not a Windows success; the correction and its independent rerun
are recorded below.

The Windows correction uses the private reader's matching directory/file
custody APIs. General directory creation does not replace an elevated token's
Administrators default owner; the strict reader requires TokenUser ownership.
The IPC owner-private directory helper repairs that ownership, and private
file creation assigns it on the exact file handle. A common read/replace/read
regression covers both publications. The lost-reply fixture now bounds accept
and request reads and reports child stdout/stderr before a server failure,
so an endpoint failure cannot conceal diagnostics behind an unbounded join.
[Hosted correction run40486233444](https://github.com/forge-ci/ghd/actions/runs/40486233444)
completed successfully at ac40666: fast, Dylint, CI OK and native x64/ARM64
Windows and macOS integration suites all passed.

The directory check also runs for existing Linux/macOS state directories.
The regression starts with an ordinary existing directory, enforces the
strict owner-private contract, and verifies private read/replace/read behavior.
Creation alone is not treated as validation of an existing path.

Review found that exclusive private-file creation cannot reuse a fixed legacy
.tmp path after interruption. Each publication now uses an exclusively owned
kernel temporary directory on the target filesystem, with private ownership,
close-before-rename and cleanup on success/error. The readback regression also
starts with an owned leftover .tmp, proves it remains untouched, and checks
that fresh staging directories are cleaned. This policy is mirrored across
native adapters rather than weakening either platform's read validator.

The corrected private-state tree passes `./ci.py all`: all Rust suites,
122 installed Python tests, backend wheel smoke and action gates. The new
existing-directory/leftover-stage/failure-cleanup regression passes. Read-only
clud-review is clean. Contract precheck reports zero violations with the
existing 17 local-only needs-review checks retained. The hosted correction run above also verifies native Windows/macOS integration
coverage alongside the required routine jobs.


## Bounded retention and restart validation (#11)

The shared LRU index accounts for all four cache namespaces, including empty
coalescing slots. Configured entry/byte limits, zero/oversized capacity and
cross-namespace eviction are regression-tested. Detached active owners are
limited separately; their completion cannot recreate an evicted slot.

Optional disk snapshots preserve only reads with validators, under the same
identity key and original write epoch. Restarts force validation before reuse;
exact CLI/raw GraphQL responses without validators are fetched again. Disk has
its own bounded capacity rather than extending the memory accounting budget.
The store uses owner-private native files and stable interprocess locking.
Schema/checksum/custody failures and publication errors are explicit. Process
interruption is covered; power-loss directory durability is not claimed.

Regression schedules include ghx #27's daemon-stopped mutation, overlapping
writes, abandoned intents, late old publishers, failed replacement and
concurrent snapshot publishers. The short freshness default and run-attempt
keys remain unchanged. Explicit per-command TTLs and an optional completed-run
extension preserve short TTLs for mutable metadata, even when stable projections
share the same body. Origin storage/freshness policy also applies to persisted
collections. A canceled restart-validation regression verifies that restored
bytes cannot stay indexed outside their payload charge. The final #11 batch passes `./ci.py all`: all Rust suites, 136 installed Python
tests, backend wheel smoke and action gates. Retained RED evidence covers the
two capacity cases and five policy cases. Read-only clud-review is clean;
contract precheck reports zero violations and the same 17 local-only review
items. Native validation of the new persistence head follows this push.


## Shared daemon budget and cooldown (#12)

Core policy uses an explicit clock, while private governor reservations/cooldown survive restart
independently of optional body snapshots. Fresh hits bypass dispatch, essential
work has protected capacity, and authoritative refusal never hides an old body.
Provider measurements and minimum dispatch estimates remain distinct. Caller
digests do not change cache identity or claim per-caller enforcement.

The #11 head bc33709 passed [hosted run40493348359](https://github.com/forge-ci/ghd/actions/runs/40493348359),
including native Windows/macOS x64/ARM64 persistence/cancellation Rust tests and
selected installed smoke checks. Its fast Python unit lane ran 132 cases and
its selected native integration lane ran four marked smoke cases. The separate
coverage correction now selects the complete installed-test directory on native
integration lanes while retaining those ordinary fast tests. That expanded
native validation will run with the next feature push.


The final #12 batch passes `./ci.py all`: all Rust suites, 144 installed Python
tests, backend wheel smoke and action gates. The focused reserve-floor RED is
retained. Read-only clud-review is clean after fixing its record-domain finding;
a valid cache envelope cannot decode as default governor policy. Contract
precheck reports zero violations and the same 17 local-only review items.

The first expanded #12 native run,
[40501114131](https://github.com/forge-ci/ghd/actions/runs/40501114131),
passed Rust/fast checks but exposed Python fixture failures: long macOS Unix
socket names, a Windows renderer wrapper ending before its delegated output,
and a test assuming immediate success above the three-routine dispatch limit.
The correction owns a short socket directory, waits for the renderer child,
and preserves sixteen-way coalescing plus a three-key race against a two-entry
cache. Local `./ci.py all` remains green with 144 Python tests and clean review.
The correction passed [run 40506918999](https://github.com/forge-ci/ghd/actions/runs/40506918999),
including all four complete native integration lanes and CI OK. The earlier
failed run remains part of the verification record.

## Repository snapshots and authority freshness (#13)

The shared CLI/Python implementation returns a schema-v1 repository envelope
with explicit pagination, truncation and freshness. Sixteen ordinary concurrent
callers share one origin request; explicit fresh calls dispatch independently
and cannot return a retained body on failure. Stale planning requires opt-in
and preserves age/source/reason. Bodies share bounded retention and mutation
coherence but are not persisted to disk. Origin HTTP policy still applies.

The focused pre-implementation failure is retained in the working verification
record. `./ci.py test` passes all Rust suites and 163 Python cases, including
19 installed snapshot cases for coalescing, authority failure, stale opt-in,
empty repositories, pagination, partial errors, head mismatch, downstream age,
storage policy, mutation invalidation and Python/CLI sharing. `./ci.py all`
passes every gate, including wheel/backend smoke and action contracts. Read-only
clud-review is clean. Contract precheck has zero violations and the same 17
local-only review items. Snapshot head 04d9f86 passed [run40510152848](https://github.com/forge-ci/ghd/actions/runs/40510152848):
104 cases on each macOS lane and 83 passed/21 existing Unix-transport skips on
each Windows lane, plus Dylint and CI OK.

## Lifecycle and cache/configuration controls (#14)

Authenticated lifecycle operations serialize starts/restarts under custody
locks, reject busy stops and wait for ownership release. Cache inspection is
redacted; flush advances the shared epoch instead of merely dropping a map.
Scoped request bypass/TTL works through CLI/Python/versioned transport without
changing another caller's defaults. Typed private JSON configuration preserves
environment precedence and rejects invalid restarts before stopping. Per-user
service examples cover foreground supervision without installing a service.

The initial installed two-case RED exposed missing cache operations and absent
request controls. The first test gate passed all Rust suites and 168 Python
cases; the subsequently extended focused operational suite passes six cases,
including active-stop refusal and flush fencing. Review found an old-daemon
policy-header compatibility gap for snapshots; a retained focused RED reproduces
it and controlled snapshots now use v3. Final `./ci.py all` passes every gate
and 170 Python cases, including all seven operations tests. Read-only review is
clean after the fix; precheck retains zero violations and 17 local review items.
Native lifecycle verification awaits the feature push.

## Bounded diagnostics and offline replay (#15)

The in-memory request ring retains safe command/caller labels, cache outcomes,
latency/size, eviction/drop counts and conservative dispatch attribution.
Opt-in capture still replaces all values, headers, variables and input paths.
Authenticated HTML/JSON views use the existing loopback endpoint; management
writes with Origin are rejected. Offline replay is bounded and exact-key-only,
with input digest and explicit fixed-TTL/model assumptions. Unknown dispatches
and provider points are never invented.

The retained two-case RED demonstrates absent ledger/replay surfaces. Focused
cases cover ordinary hit/miss, coalescing/callers, redacted bounded capture,
304/refusal/eviction, dashboard authentication/cross-origin refusal and offline
model provenance. Review exposed modeled mutation reuse (retained RED: one
modeled hit versus expected zero); explicit kind/eligibility now excludes
writes. A further authority RED reproduces the same error for forced fresh
reads; those and scoped/forced REST controls are now ineligible. Recorded writes
clear modeled retention without inferring resource relationships.
Final `./ci.py all` passes all gates and 179 Python cases. Read-only review is
clean; precheck has zero violations and 17 existing local review items.

## Native lifecycle correction after #14

Run40512325192 passed fast, Dylint and both macOS integration lanes, but both
Windows integration lanes timed out after the first two operational tests;
CI OK failed. This is not a passing result. The third case captures concurrent
launcher output while a daemon survives it. The old Windows std spawn can
inherit unrelated capture handles into that surviving daemon, retaining EOF.
The platform facade now uses kernal-api's existing synchronous daemon boundary,
which passes only explicit regular log handles and null stdin. The old native
spawn copies are removed, and the lifecycle regression stays enabled.
Native integration also dumps Python thread stacks after a stuck case exceeds
60 seconds; test assertions and job deadlines remain unchanged. Local testing
passes all Rust suites and 179 Python cases, backend smoke/action gates, and
format/LOC checks. An unused-import lint from the initial run was removed;
corrected clippy, Ruff and pylint pass. Precheck has zero violations and the
same 17 review items; read-only review is clean. Full native verification is
pending.

The correction's [run40516857808](https://github.com/forge-ci/ghd/actions/runs/40516857808)
confirmed concurrent lifecycle completion on Windows; it no longer hangs. Both
Windows lanes still failed a config test because its parent state directory
was not protected, and ARM64 exposed a transient endpoint-sharing read during
fixture publication. The config now lives inside the fixture's already-private
identity directory; startup observation retries only missing/sharing errors
within the same deadline. Product custody checks and assertions stay intact.

The broad ci-full tag also selected musl wheels on glibc native runners, which
failed import. Musl wheel execution belongs to the existing release smoke flow.
The next run explicitly selects Linux ARM64, Windows and macOS plus routine
Linux x64 integration, preserving the six native OS/architecture lanes. The
failed broad run is not reported as passed; the planner mismatch is tracked
in [#19](https://github.com/forge-ci/ghd/issues/19). Local fixture correction
passes `./ci.py all` with 179 Python cases, clean review and zero precheck
violations (the same 17 local review items).
