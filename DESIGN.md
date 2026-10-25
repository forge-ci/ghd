# ghd: one fast, cached, concurrent gateway for all GitHub reads/writes

Status: design draft (2026-10-24). Name: `ghd` (gh daemon); PyPI `ghd`, crates.io `ghd`, repo `forge-ci/ghd`.

## 1. Why (evidence from the agent logs)

Source: a scan of the 65 project dirs under `~/.claude/projects` (467 sessions mention GraphQL).

- The same failure keeps coming back. These messages appear in about 300 sessions:
  - `API rate limit exceeded for user ID 6856673`: 124 times.
  - `GraphQL: API rate limit already exceeded for user ID 6856673`: 48 times, plus 78 times without the prefix.
  - Agents warning each other that the "5,000/hr shared across all tools and agents" budget is exhausted: 55+ times.
- These calls spend the budget (count of `gh` command invocations across all logs):

  | command | calls | backend |
  |---|---|---|
  | `gh pr view` | 9,552 | **GraphQL** |
  | `gh run view` | 9,103 | REST |
  | `gh pr list` | 5,862 | **GraphQL** |
  | `gh pr checks` | 5,246 | **GraphQL** |
  | `gh run list` | 4,497 | REST |
  | `gh issue view` | 4,297 | **GraphQL** |
  | `gh api repos/...` | 4,061 | REST |
  | `gh pr merge` | 3,310 | GraphQL mutation |
  | `gh api graphql` | 2,403 | **GraphQL** (2,208 are `{repository(owner…` reads) |
  | `gh issue list` | 1,966 | **GraphQL** |
  | `pr_merge_watch` (mentions) | 14,440 | uses both |

- `gh pr view --json` mostly asks for the same few fields: `state` (5,247), `mergeStateStatus` (1,896), `statusCheckRollup` (1,547), `mergeable`, `title`, `headRefOid`, `headRefName`, `commits`, `number`, `body`, `files`, `mergedAt`. That makes it a small, closed schema that is easy to cache.

### What already exists: clud's `gh_broker` (#1743)

`~/dev/clud/crates/clud-bin/src/gh_broker/` is about 5.4k lines. It includes a redb ETag cache, single-flight, TTL, page-by-page revalidation, scope-tagged invalidation, a rate-limit floor and watch subscriptions. **It deliberately does not broker GraphQL or porcelain commands** (`classify.rs:90-98`, `docs/architecture/gh-read-broker.md:24`). Those are the top six consumers in the table above. That is the gap this tool fills. Section 9 covers whether to lift the broker's pieces or replace it.

## 2. Goals

1. **All GitHub traffic** from agents, clud, `pr_merge_watch` and CI scripts goes through one per-user daemon. This covers reads and writes, REST and GraphQL.
2. **Cache first.** A repeated read costs 0 budget points whenever freshness can be proven cheaply.
3. **Concurrency without stampedes.** Identical in-flight reads collapse into one (single-flight). Different reads on the same repo merge into one GraphQL query (batching).
4. **Never hit the wall.** A budget governor queues and prioritizes work instead of failing, and keeps a reserve for writes and merges.
5. **Drop-in.** `ghd gh <args>` (and a `gh` shim on PATH) gives byte-identical output for the top commands and falls back to the real `gh` for everything else.
6. **Blazing fast.** Warm cache answers in under 5 ms. No `gh` subprocess on the hot path.

Non-goals: replacing `gh` auth, and interactive or TTY flows (`gh pr create` editor, `gh auth`).

## 3. Shape: a Rust tool shipped through PyPI, modeled on **soldr** (and obs-rust/obs-studio's newer repo conventions)

`template-python-rust-cmd` sets the baseline, but `~/dev/soldr` has the newer architecture and is the closest analog. It is a Rust CLI and a long-lived daemon with a cache, shipped on PyPI with `build-backend = "soldr"`. So the crate split follows soldr's internal `publish = false` crates (`soldr-cli`, `soldr-daemon`, `soldr-cache`, `soldr-core`, `soldr-fetch`, `soldr-platform`):

```
crates/
  ghd-cli/        # [[bin]] ghd: client, gh-compat argv, output formatting  (cf. soldr-cli)
  ghd-daemon/     # router, single-flight, governor, watches                  (cf. soldr-daemon)
  ghd-cache/      # redb store + memory LRU, scope tags (lifted from gh_broker) (cf. soldr-cache)
  ghd-fetch/      # HTTP/2 REST+ETag client, GraphQL batcher/splitter         (cf. soldr-fetch)
  ghd-core/       # request model, cache keys, budget model, IPC protocol     (cf. soldr-core)
  ghd-platform/   # daemon spawn, port/token file, paths                      (cf. soldr-platform)
  ghd-py/         # PyO3 _native client for python callers (pr_merge_watch)
src/ghd/       # python package
pyproject.toml      # build-backend = "soldr", manifest-path = crates/ghd-cli/Cargo.toml
```

Repo and CI conventions to copy from soldr and obs-studio, not the older template:
- `ci.toml`, `local-gate.toml`, `bosn.toml` and `ci-attestations.yml`, plus the `ci/` and `lint/` dirs and `dylints/`.
- `deny.toml`, `rust-toolchain.toml`, and `contracts/` for the IPC and gh-output contracts.
- `CLAUDE.md`, `DESIGN.md` and `RELEASE.md`, as in soldr.
- From obs-studio (#2056): every third-party dependency is pinned with `=` to its Cargo.lock version, with no default features, in `[workspace.dependencies]`. Edition 2024, `resolver = "3"`, and `publish = false` on internal crates.
- Workspace clippy lints are the same as soldr's: `unwrap_used = deny` and so on.

Seeding the repo means copying soldr's CI skeleton, not the template's.

## 4. Architecture

```
 agents / clud shim / pr_merge_watch / python
        │  (loopback HTTP/1.1 keep-alive, 127.0.0.1, bearer token, protobuf bodies)
        ▼
 ┌────────────────── ghd daemon (one per user+token-hash) ──────────────────┐
 │ request router ─► single-flight table ─► cache (redb, memory LRU on top)   │
 │       │                                   ▲  hit + proven fresh → return   │
 │       ▼ miss/stale                        │                                │
 │ freshness prober (REST ETag 304s, events feed) ──┘                         │
 │       ▼ really changed                                                     │
 │ GraphQL batcher (5–20 ms window, alias-merge per repo)                     │
 │ REST pool (HTTP/2, keep-alive, reqwest+rustls)                             │
 │       ▼                                                                    │
 │ budget governor (core + graphql buckets, from X-RateLimit-* headers)       │
 │ write path: execute → targeted invalidation (scope tags) → wake watchers   │
 └────────────────────────────────────────────────────────────────────────────┘
```

- **Foundation: `kernal-api`** (`~/dev/kernal-api2`, the shared systems facade for soldr, zccache and fbuild). The app uses only the facade, never Tokio or `running-process` directly. A boundary Dylint enforces that. Pin the exact release (currently 0.1.2). It covers:
  - `async_engine`: the runtime, cooperative cancellation, and separate connect and transfer-idle timeouts.
  - `http`: a bounded client that keeps raw wire bytes, so ETags and conditional requests stay exact.
  - `daemon-identity`, `daemon-frame-v1` and `daemon-registration-v2`: daemon identity, endpoint, framing and registration.
  - `independent-spawn`: launches the daemon so it outlives the agent that started it.
  - `sqlite`: the store, in place of redb. The schema and SQL stay with the app.
  - `http_server`: the daemon's loopback HTTP listener. `http` is the client side.
  - `hash`: BLAKE3 cache keys.
- **Client↔daemon is loopback HTTP, not a socket or pipe.** The daemon listens on `127.0.0.1:<port>` and writes the port plus a per-session bearer token to an owner-private file, read with `read_private_regular_file_bounded`. clud's `gh_broker` already works this way (`READ_PATH`, `INVALIDATE_PATH`, `WATCH_PATH`, `LEDGER_PATH`).
  - Routes: `POST /v1/read`, `POST /v1/write`, `POST /v1/invalidate`, `POST /v1/watch` (a long poll), `GET /v1/ledger`, `GET /v1/stats` and `GET /v1/health`.
  - Python callers and plain `curl` can use it with no native extension, so `ghd-py` becomes optional.
  - Request and response bodies are Protocol Buffers (`application/x-protobuf`), with `.proto` files in `contracts/`. That keeps it inside kernal-api's PROTOCOLS.md rule that daemon communication is protobuf, never JSON. `GET /v1/stats` and `/v1/ledger` may also serve JSON for humans.
- **Caveat:** kernal-api's generic broker-daemon "managed service" has not been moved out of `running-process` yet (AGENTS.md, PROTOCOLS.md). Until it is, the app owns its own daemon lifecycle on top of the identity, frame and registration slices. It moves to the managed service once that exists.
- **Transport:** talks to `api.github.com` directly with tokio + reqwest/hyper over HTTP/2. It reads the token once via `gh auth token`, or from `GH_TOKEN`/`GITHUB_TOKEN`. Cache keys hash the token, following `gh_broker::FORWARDED_ENV`, so two identities never share entries.
- **Daemon lifecycle:** the first client call auto-spawns the daemon. It is kept alive by an idle timeout. A lock file stops two daemons running for the same identity.

## 5. The levers that make it cheap

### 5.1 Proving freshness for free

GitHub's docs say a conditional REST request that returns **304 Not Modified does not count against the primary rate limit**. GraphQL has no ETags. So every GraphQL-backed entity gets a cheap REST "sentinel":

| entity | sentinel (conditional GET) |
|---|---|
| PR state / mergeability / head | `GET /repos/{o}/{r}/pulls/{n}` (ETag) |
| checks rollup | `GET /repos/{o}/{r}/commits/{sha}/check-suites` + `/status` (ETag) |
| issue | `GET /repos/{o}/{r}/issues/{n}` |
| PR/issue lists | `GET /repos/{o}/{r}/pulls?state=…&sort=updated&per_page=1` |
| anything in a repo | `GET /repos/{o}/{r}/events` (ETag, honors `X-Poll-Interval`): one feed that invalidates many keys |

A cached GraphQL result is served if its sentinel returns 304. A 200 marks the result stale, and only then does it pay for GraphQL.

### 5.2 GraphQL batching (alias merge)

Concurrent requests for the same repo inside a short window (5–20 ms, tunable) become one query, for example:

```graphql
{ repository(owner:"o",name:"r"){
    p1:pullRequest(number:12){...PRFields}
    p2:pullRequest(number:15){...PRFields}
    i1:issue(number:40){...IssueFields} }
  rateLimit{cost remaining resetAt} }
```

The fields are the **union** of what the waiting callers asked for. The response is split back per caller. 20 parallel agents each running `gh pr view` cost about 1 point instead of 20. `rateLimit{}` rides along on every query so the governor always knows the real remaining budget.

### 5.3 Single-flight and cache tiers

The single-flight key is `(identity, normalized request, field set)`. Field sets are normalized so that `state,title` is served from a cached `state,title,body`.

There are two tiers: an in-memory LRU over redb, which persists across daemon restarts. Each kind of entity gets its own TTL:
- merged/closed PRs and completed runs are **frozen**, with an infinite TTL. `gh_broker` already does this for listings.
- in-flight checks: 10–30 s plus a sentinel.

### 5.4 Writes and invalidation

Mutations such as `gh pr merge`, `comment`, `edit`, `run rerun`, `workflow run` and `api graphql mutation` go through the daemon. After each one, the daemon invalidates the affected entities using the scope tags from `gh_broker/scope.rs` and wakes their watchers, so the next read never shows pre-write state.

### 5.5 Budget governor

- It tracks the `core` and `graphql` buckets separately from the `X-RateLimit-*` headers and from `rateLimit{}`.
- Each request has a priority class: `write` > `interactive read` > `watch/poll` > `prefetch`.
- When remaining budget drops below a floor (default 10%), only writes and interactive reads go out. Polls stretch out, and callers **wait in a queue** instead of getting a rate-limit error.
- **Secondary limits:** it caps concurrent upstream requests (default 8), spaces mutations by at least 1 s, and honors `Retry-After`.
- A ledger, like `gh_broker`'s `/gh/ledger`, records the cost of every call (`ghd stats`), so the next audit isn't a log grep.

### 5.6 Watches instead of loops

`ghd watch pr 123 --until merged|checks-done|state-change` holds one shared subscription that the events feed and sentinels drive. 50 agents watching one PR cost the same as 1. This replaces hand-written polling and becomes the engine under `pr_merge_watch` and `clud ci`.

## 6. CLI surface

```
ghd gh <gh args…>        # gh-compatible; brokered when understood, else exec real gh
ghd pr view|list|checks  # native fast paths (same flags/--json/--jq as gh)
ghd run view|list|watch
ghd issue view|list
ghd api [graphql] …      # REST GET → ETag cache; graphql query → batch/cache; mutation → write path
ghd watch …              # subscriptions
ghd stats | ghd ledger # budget + per-caller cost
ghd daemon [--foreground]
```

**gh compatibility is a hard requirement.** Golden tests record real `gh` output for each supported command and field combination (from the field histogram in section 1) and diff it against ghd's output. Formatting for `--jq` and `--template` follows `gh_broker`'s replay trick, or uses `jaq` in-process for speed.

## 7. Rollout

1. **M1:** daemon, REST ETag cache and `ghd api` (port the `gh_broker` store and service). Parity with clud.
2. **M2:** GraphQL batcher, sentinels, and native `pr view/list/checks` and `issue view/list`. This covers about 70% of the GraphQL spend.
3. **M3:** write path and invalidation, then `run view/list`, then `watch`. `pr_merge_watch` moves to `ghd` (PyO3).
4. **M4:** a `gh` shim on PATH for agent sessions. The clud shim routes to ghd. `ghd stats` must show at least a 10× drop in upstream points per agent-hour.

## 8. Success metrics

- Zero "API rate limit exceeded" errors in agent logs over a week of normal grind load.
- Warm hit under 5 ms (p50), and a cold `pr view` no slower than `gh`.
- Upstream GraphQL points per `gh pr view` invocation at or below 0.1, measured by the ledger.

## 9. Open decisions

1. **Where it lives:** a standalone repo (this dir, repo `forge-ci/ghd`, published to PyPI as `ghd`), or a crate inside clud? Recommendation: standalone. clud depends on it, and lifts `gh_broker`'s store, scope and collection code into `ghd-store`/`ghd-core` instead of keeping two caches.
2. **Direct HTTP vs. spawning `gh`:** the recommendation is direct HTTP for speed. The cost is re-implementing `gh`'s output formatting for the supported commands. Unsupported commands still exec `gh`.
3. **Multiple machines/tokens:** the daemon is per machine. Cross-machine sharing, for example a small HTTP cache server, is deferred.

## 10. Spike results (2026-10-24, `crates/ghd`)

The spike covers `ghd pr view <n> -R o/r --json …` (20 scalar fields), `ghd stats` and `ghd daemon`. It is built on kernal-api 0.1.31 (`http-client`, `http-server`, `json`). Client↔daemon traffic is protobuf over loopback HTTP with a bearer token. The client is plain std, with no runtime or TLS.

Live test against `talos-hq/clud`, ten concurrent reads per round (5 PRs, each read twice):

| | upstream cost | answered as |
|---|---|---|
| round 1 (cold) | 5 REST sentinels + **2 GraphQL points** (gh: ~10 points) | 5 fetched, 5 `fresh` behind the per-PR lock |
| round 2 (warm) | **0** (core_remaining unchanged; 304s are free) | 5 revalidated, 5 fresh |

- A warm read takes 15 ms with ghd against 576 ms with `gh`. The fields match `gh` exactly; only the whitespace differs.

Findings to carry into the real build:
1. Batches still split in two, because sentinel latencies vary by more than the 50 ms window. Fix: batch the sentinels too, or start GraphQL as soon as the first sentinel returns 200.
2. kernal-api's `http::Client` forces HTTP/1 with `pool_max_idle_per_host(0)`, so every upstream call pays a fresh TLS handshake. Ask kernal-api for a keep-alive/HTTP2 option.
3. Output whitespace must follow `gh`'s TTY rule: compact when piped, indented on a TTY.
4. Daemon start is serialized with a `start.lock` flock. Without it, ten cold clients started ten daemons.
5. Renamed repos answer 301 (`zackees/clud` → `talos-hq/clud`), so redirects are on, max 3.
6. The daemon has no idle exit yet, and the cache is in memory only (sqlite comes next).
