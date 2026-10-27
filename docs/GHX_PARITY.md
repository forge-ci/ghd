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
| #7 | Host and credential isolation | Implemented and verified; awaiting integration PR |
| #8 | gh passthrough and mutation coherence | Pending |
| #9 | Expanded resource and collection reads | Pending |
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
