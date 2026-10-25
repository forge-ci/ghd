# `crates/private/ghd-daemon/src/`

The long-lived side of ghd. Built on kernal-api only (async engine,
HTTP client and server, JSON, secure random); nothing here names tokio,
reqwest or hyper.

## Files

| File | What it owns |
|---|---|
| `lib.rs` | `pub mod server; pub mod upstream; pub use server::run;` |
| `server.rs` | `run()`, routing, auth, the per-PR cache and `/v1/stats`. |
| `upstream.rs` | `Upstream`: REST sentinels, the GraphQL batcher, `Counters`, `build_query()`, `split()`. |

## `server.rs`

- `run()` gets a GitHub token (`GH_TOKEN`, `GITHUB_TOKEN`, else
  `gh auth token`), builds a multi-thread runtime, binds
  `127.0.0.1:0`, mints a 16-byte hex bearer token, and writes
  `port token` with `ghd_core::endpoint::write`.
- `route()` rejects any request without `Authorization: Bearer
  <token>` (401), then serves `GET /v1/health`, `GET /v1/stats` and
  `POST /v1/pr/view`; anything else is 404.
- `pr_view()` decodes `PrViewRequest`, validates owner/repo and every
  field against `ghd_core::fields` (400 on failure), takes the PR's
  slot lock, and returns pretty-printed JSON of just the requested
  fields in a `PrViewResponse`. Upstream failures are 502.
- `read()` decides the `source`: `fresh` (validated < 2 s ago),
  `revalidated` (sentinel 304), or `fetched` (GraphQL).

## `upstream.rs`

- `sentinel(path, etag)` — conditional GET with `If-None-Match`;
  returns `(status, etag)` and stores `x-ratelimit-remaining`.
- `pull_request(owner, repo, number, fields)` — joins the open batch
  for that repo or opens one and schedules `flush` after the window
  (50 ms). Waiters get the result over a broadcast channel.
- `build_query()` aliases every PR (`p<n>: pullRequest(number: n)`)
  with its union of fields and adds `rateLimit{cost remaining}`;
  `split()` hands each alias back to its PR.
- Redirects are followed (max 3) because renamed repos answer 301.

## Why per-PR locks plus per-repo batches

The lock collapses identical concurrent reads into one upstream trip;
the batch merges *different* PRs of one repo into one GraphQL query.
Together, ten parallel agents reading five PRs cost five free-or-cheap
sentinels and one query.

## Testing

`upstream.rs` has unit tests for query aliasing and splitting
(`ghd-daemon:lib`). Nothing here is tested against live GitHub.

```bash
soldr cargo test -p ghd-daemon --locked
```
