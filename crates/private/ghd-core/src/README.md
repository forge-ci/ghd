# `crates/private/ghd-core/src/`

Everything on the caller's side of the loopback daemon. No async
runtime and no TLS live here, so a warm `ghd` invocation costs one
loopback round trip.

## Files

| File | What it owns |
|---|---|
| `lib.rs` | Module index, `pub(crate) use ghd_platform as platform;`, and `version_banner()` (`ghd <version> (<os>)`). |
| `proto.rs` | prost messages: `PrViewRequest { owner, repo, number, fields, hostname }` and `PrViewResponse { json, source, error }`. Tags are stable — retire with `reserved`, never reuse. |
| `identity.rs` | Host-bound credentials, canonical repository selectors, fingerprinted daemon directories and deployment API URLs. |
| `endpoint.rs` | `Endpoint { port, token }`; `state_dir()`, `parse()`, `read()`, `write()` for `<state root>/identities-v1/<digest>/endpoint` (`port token`), written owner-private. |
| `client.rs` | `call(method, path, body)`: one std `TcpStream` HTTP/1.1 request with the bearer token; `parse_response()` (Content-Length or chunked). Starts the daemon when nothing answers or the token is rejected (401). |
| `cli.rs` | `run(args)` for every subcommand except `daemon`; `parse_pr_view()`, `pr_view_request()`, `owner_repo_from_url()`, `USAGE`. |
| `fields.rs` | `PR_FIELDS` (the 20 `gh pr view --json` scalars ghd answers) and `valid_name()` (safe owner/repo names). |

## How a `ghd pr view` flows through here

1. `cli::parse_pr_view` turns `<n> [-R owner/repo] --json a,b` into a
   `PrViewRequest`. Without `-R`, the repo comes from `git remote
   get-url origin` (`git@`, `https://` and `ssh://` GitHub forms).
2. `cli::pr_view_request` encodes it and `client::call("POST",
   "/v1/pr/view", ...)` sends it.
3. If the endpoint file is missing, the daemon does not answer, or it
   answers 401, `client` takes `<state dir>/start.lock`, rechecks
   `/v1/health`, spawns `ghd daemon` detached (stdout/stderr to
   `daemon.log`), and waits up to 5 s for it to become healthy.
4. The reply is decoded; a non-empty `error` becomes an `io::Error`.
   `GHD_DEBUG` prints the `source` (`fresh`/`revalidated`/`fetched`).

The daemon executable is `$GHD_EXE`, else the current process if it is
`ghd`, else `ghd` on PATH (the Python binding's case).

## Why `fields` lives on the client side

The daemon builds its GraphQL selection only from `PR_FIELDS` and
checks owner/repo with `valid_name`, so this list is also the injection
allowlist. Both crates need it; `ghd-daemon` depends on `ghd-core`, not
the other way round.

## Dependency direction

`ghd-core → ghd-platform`. OS access (state dir, private files,
detached spawn, exe suffix) goes only through `crate::platform::*`.
Depends on `anyhow` and `prost`; never on `ghd-daemon` or kernal-api.

## Testing

Unit tests sit in `#[cfg(test)] mod tests` in `lib.rs`, `endpoint.rs`,
`client.rs`, `cli.rs` and `fields.rs` (`ghd-core:lib` in `ci.toml`).
None touch the network: argument parsing takes an injectable
default-repo function, and response parsing works on byte strings.

```bash
soldr cargo test -p ghd-core --locked
```

## Host and credential resolution

`identity.rs` owns GitHub hostname validation, host-bound credential
precedence, repository selector parsing and API URL construction. Every client
call resolves an identity before it reads an endpoint. Endpoint directories
are derived from the credential/host digest, and every loopback request carries
that digest alongside the per-session bearer token. Raw credentials stay in
memory and the daemon spawn environment; they never enter paths or logs.

## Compatibility and write custody

`operation.rs` parses GraphQL operation selection. `compat.rs` owns real-gh
lookup, argument/stream forwarding and conservative command classification.
`coherence.rs` fences cached reads through a cross-process protobuf journal.
A pre-dispatch intent remains unresolved on uncertain/failed execution; its
presence prevents a restart from restoring stale cache authority. A connected
client request is never replayed automatically after its reply is lost.

Resource reads use `resource.rs` for target binding, field/filter/limit
normalization and nested-order-preserving projections. `reads.rs` supplies
the shared CLI/Python protobuf request, explicit backend/context and read-only
API. The compatibility executor remains the owner of unsupported/interactive
forms; writes still enter the journal before real gh dispatch.
