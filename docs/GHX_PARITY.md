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
| #10 | General REST cache and revalidation | Pending |
| #11 | Bounded durable cache and restart invalidation | Pending |
| #12 | Shared rate governor and cooldown | Pending |
| #13 | Repository snapshots and authority freshness | Pending |
| #14 | Daemon/cache operations and configuration | Pending |
| #15 | Diagnostics, dashboard and replay | Pending |
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
synthetic cache tests run on every declared platform. No live GitHub answers
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
