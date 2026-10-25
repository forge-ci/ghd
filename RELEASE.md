# Releasing ghd

A release is a version bump. Nobody runs a release script.

## Cutting a release

1. In one commit, bump `[workspace.package].version` in `Cargo.toml` and
   `version` in `pyproject.toml` to the same new semver, and run
   `soldr cargo update -w` so `Cargo.lock` follows.
2. Merge it to `main`.

The rest happens on its own:

```
push to main (bump)
  └─ auto-release.yml  prepare ── "candidate" ──► gh workflow run ci.yml -f sha=<bump>
ci.yml release flow (workflow_dispatch, titled "release <sha>")
  release-guard → every platform wheel → smokes → release-verify → publish (OIDC proof)
  └─ release-handoff ──► gh workflow run auto-release.yml
                           -f candidate_sha=<bump> -f full_ci_run_id=<this run>
auto-release.yml  prepare ── "publish" (run checked through the API)
  ├─ github-release: tag v<version> at <bump>, attach wheels + sdist
  └─ publish:        the same bytes → PyPI (trusted publishing)
```

- **Exact bytes.** Both publishing jobs download `release-dist-verified`
  from the verifying run by id. Nothing is rebuilt after verification
  (ci.yml policy REL-005).
- **Fails closed.** `ci/auto_release.py` refuses to publish if any of
  these is true:
  - the run is not a `ci.yml` `workflow_dispatch`;
  - the run is not titled `release <sha>` for this exact commit;
  - `release-verify` did not succeed in that run;
  - the tag already exists;
  - `Cargo.toml` and `pyproject.toml` disagree on the version.

  Every git or API error aborts the release. The decision table is
  unit-tested in `tests/test_auto_release.py`.
- **`0.0.0` is never released.** It is the name reservation on PyPI and
  crates.io.
- **No `workflow_run`.** Every hop is an explicit dispatch, as required by
  ci.yml policy SEC-003.

## Re-running a stuck release

- **The release flow failed:** fix it on main. If the version is
  unchanged, dispatch it again by hand:
  `gh workflow run ci.yml -f sha=<bump sha>`.
- **The release flow succeeded but publishing failed:** start the last hop
  directly. A tag that already exists is never overwritten.

  ```sh
  gh workflow run auto-release.yml -f candidate_sha=<bump sha> -f full_ci_run_id=<green run id>
  ```

## One-time setup (repository owner)

| where | what |
|---|---|
| GitHub → Settings → Environments | create `pypi` and restrict its deployment branches to `main` (ci-lint SEC-006) |
| PyPI → `ghd` → Publishing | add a trusted publisher: owner `forge-ci`, repository `ghd`, workflow `auto-release.yml`, environment `pypi` |
| GitHub → Settings → Actions | allow GitHub Actions to create releases (the `contents: write` token on `github-release`) |

No repository or environment secrets exist or are needed (ci-lint SEC-005).

## crates.io

`ghd` 0.0.0 on crates.io is a **name reservation**. Under the ci.yml
`rust-pypi-app` layout (RUST-011), the published amalgam `crates/ghd`
depends on `publish = false` private crates, so `cargo publish` cannot
upload it yet. Shipping the crate needs one of two changes:

- publish the private crates under `ghd-*` names, which needs a RUST-011
  exception; or
- vendor them into the amalgam at package time.

Until then the binary ships through PyPI wheels and GitHub Releases.
