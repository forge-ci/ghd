# ghd

**One cached, batched, budget-aware gateway for `gh` reads.**

[![ghd demo: ten concurrent gh pr view reads served from cache, batched GraphQL and free ETag revalidation](docs/assets/demo.gif)](https://forge-ci.github.io/ghd/)

▶ **[Play the interactive demo](https://forge-ci.github.io/ghd/)**: a real
recorded session against GitHub, with chapters and the numbers it measured.

Agents that run `gh pr view`, `gh pr checks` and `gh api graphql` in loops
share one 5,000-point/hour GitHub budget, and they routinely exhaust it
(`GraphQL: API rate limit already exceeded for user ID …`). `ghd` puts a
per-user daemon in front of GitHub:

```console
$ ghd pr view 2138 -R talos-hq/clud --json number,state,title,mergedAt
{
  "mergedAt": "2026-10-24T21:12:09Z",
  "number": 2138,
  "state": "MERGED",
  "title": "fix(gc): retry a locked registry …"
}
```

The output has the same fields and values as `gh pr view --json`. A fresh
read takes about 5 ms; `gh` takes about 480 ms (both measured in the demo).

## How it saves budget

A read is answered by the cheapest step that can prove the data is fresh:

| step | cost | when |
|---|---|---|
| `fresh` | 0 | validated in the last 2 s; concurrent identical reads wait on one per-PR lock |
| `revalidated` | **0** | the PR's REST ETag returns `304 Not Modified`, which GitHub does not count against the rate limit |
| `fetched` | ~1 GraphQL point **per batch** | every PR of one repo requested within 50 ms goes into **one** aliased GraphQL query |

Measured in the spike (10 concurrent reads = 5 PRs × 2): the cold round
cost 2 GraphQL points (`gh` would spend ~10), and the warm round cost
0, with the REST counter unchanged.

## Install

```sh
pip install ghd        # or: uv tool install ghd
```

The wheel bundles the native `ghd` binary (no Python launcher in front of
it) and a PyO3 extension for Python callers:

```python
from ghd.bindings import pr_view
pr_view("talos-hq/clud", 2138, ["state", "mergeable"])
```

## Usage

```text
ghd pr view <number> [-R owner/repo] --json f1,f2   like `gh pr view --json`
ghd stats                                           what the daemon saved
ghd daemon                                          run the daemon (auto-started)
ghd --version | --help
```

- Without `-R`, the repository comes from the `origin` remote, as with `gh`.
- `GHD_DEBUG=1` prints which step answered each read.
- The daemon starts itself on first use. One start lock means concurrent
  cold clients start exactly one daemon.
- The daemon authenticates with `GH_TOKEN`, `GITHUB_TOKEN` or `gh auth token`.
- Supported `--json` fields so far are the plain scalars: `additions`,
  `baseRefName`, `body`, `changedFiles`, `closed`, `closedAt`, `createdAt`,
  `deletions`, `headRefName`, `headRefOid`, `id`, `isDraft`,
  `mergeStateStatus`, `mergeable`, `mergedAt`, `number`, `state`, `title`,
  `updatedAt` and `url`. See [DESIGN.md](DESIGN.md) for the roadmap:
  checks, lists, issues, writes and watches.

## Architecture

```
ghd (CLI, std-only client) --loopback HTTP + protobuf + bearer token--> ghd daemon
                                                          |
                     per-PR cache -> REST ETag sentinel -> batched GraphQL -> api.github.com
```

| crate | role |
|---|---|
| `crates/ghd` | the public amalgam: `pub use` re-exports only; feature `daemon` |
| `crates/ghd-cli` | the `ghd` binary (bundled into the wheel by soldr) |
| `crates/ghd-py` | the PyO3 `_native` extension (client side only) |
| `crates/private/ghd-core` | wire protocol, endpoint file, loopback client, gh-compatible CLI |
| `crates/private/ghd-daemon` | loopback server, cache, sentinels, GraphQL batcher |
| `crates/private/ghd-platform` | the only crate with host-specific code (`cfg_select!` once) |

- The daemon is built on [kernal-api](https://crates.io/crates/kernal-api)
  (async engine, HTTP client and server, JSON, secure random). No crate
  names tokio, reqwest or hyper directly.
- Every third-party dependency is pinned with `=`.
- More detail: [DESIGN.md](DESIGN.md), [crates/README.md](crates/README.md)
  and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Development

```sh
./test        # soldr cargo test --workspace, build + install the wheel, pytest
./lint        # rustfmt, clippy -D warnings, ruff
uv run --no-project --script ci/local.py precheck   # ci-lint precheck (also a hook)
```

CI is declared, not scripted. [`ci.toml`](ci.toml) (schema 3, profile
`rust-pypi-app`) is the contract, checked by
[`ci-lint`](https://github.com/zackees/ci.yml). This repo was instantiated
from the fleet reference repo
[`zackees/template-python-rust-cmd`](https://github.com/zackees/template-python-rust-cmd),
so its gates, hooks and entry points are described in [CLAUDE.md](CLAUDE.md).

## Releases

Releases are automatic: merge a commit to main that bumps
`[workspace.package].version` in `Cargo.toml` and `version` in
`pyproject.toml`. It then builds every platform's wheel, verifies them,
tags the commit and publishes to PyPI with no stored secrets. See
[RELEASE.md](RELEASE.md).

## License

MIT
