# `ghd-daemon`

The long-lived side of ghd: a loopback HTTP server that answers
`gh pr view --json` reads from a per-PR cache, proves freshness with
free REST ETag 304s, and batches GraphQL per repo. Wired into the
public `ghd` amalgam with plain `dep:` feature gating
(`daemon = ["dep:ghd-daemon"]`) and re-exported as `ghd::run_daemon` —
never a `#[cfg(feature = ...)]` branch inside shared logic.

`publish = false`: see `crates/private/README.md` for the rules every
crate under `crates/private/` follows.

## Why it is its own crate

Everything that needs an async runtime, TLS, an HTTP server and JSON
lives here, so `ghd-core` (and every short-lived `ghd` invocation, and
`ghd-py`) stays std-only. A consumer that only talks to a running
daemon builds the `ghd` amalgam without `daemon` and never compiles
this crate.

## Modules

- `server` — `run()`: reads the GitHub token (`GH_TOKEN`,
  `GITHUB_TOKEN`, else `gh auth token`), binds `127.0.0.1:0`, mints a
  random bearer token, writes `port token` via `ghd_core::endpoint`,
  and serves:
  - `GET /v1/health` → `ok`
  - `GET /v1/stats` → JSON counters (reads, served fresh, sentinel
    304/200, GraphQL queries/PRs/points, remaining budgets)
  - `POST /v1/pr/view` → protobuf `PrViewRequest` / `PrViewResponse`
  Every route requires `Authorization: Bearer <token>` (else 401).
- `upstream` — `Upstream`: `sentinel()` (conditional GET with
  `If-None-Match`; records `x-ratelimit-remaining`), `pull_request()`
  (joins or opens a per-repo batch), `build_query()` / `split()` (one
  alias-merged GraphQL query per repo, fields = union of callers'
  sets, results split back per PR), and `Counters`.

## How a read is answered

Cheapest first, each PR behind its own async lock so concurrent
identical reads make one upstream round trip:

1. `fresh` — validated within `FRESH_FOR` (2 s) and the entry covers
   the requested fields: no upstream call.
2. `revalidated` — the PR's REST sentinel
   (`/repos/{o}/{r}/pulls/{n}`) returned 304: costs no budget.
3. `fetched` — the batched GraphQL reader (`BATCH_WINDOW` = 50 ms),
   asking for the requested fields plus everything already cached, so
   an entry only grows.

The sentinel runs before GraphQL, so stored data is never older than
the ETag it is stored under. Owner/repo and field names are checked
against `ghd_core::fields` before anything is spliced into GraphQL.
The cache is in memory only; there is no idle exit yet.

## Dependencies

`ghd-core`, `kernal-api` (async engine, HTTP client/server, JSON,
secure random) and `prost`. No crate here names tokio, reqwest or
hyper.

## Testing

Unit tests live under `#[cfg(test)] mod tests` in `upstream.rs`
(query aliasing and splitting) — `ghd-daemon:lib` in `ci.toml`'s
`[rust.tests].binaries`. `[lib] doctest = false`. No test talks to
GitHub.

```bash
soldr cargo test -p ghd-daemon --locked
```
