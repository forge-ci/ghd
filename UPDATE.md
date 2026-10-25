# Update Procedure


Default `python3 ci/local.py act` requires a clean committed tree, uses the
shared gate's execution or fresh-result reuse, and amends HEAD on success.
See the [local-loop contract](CLAUDE.md#ci).
Explicit title/job selections remain diagnostics; native and release coverage
remain required.

When changing repo structure, CI gates, release flow, or agent
guidance, walk this checklist so nothing rots out of sync.

## Repo structure / CI

1. Update the relevant manifest, gate, or script. Any Rust/wheel
   command in a gate or script goes through `soldr` — never bare
   `cargo`/`maturin`.
2. If you added a gate:
   - `ci/gates/<name>.py` with `def run() -> int`.
   - Register in `ci.py::GATE_ORDER`.
   - Add a step to `.github/workflows/ci.yml` **once that file
     exists** — `zackees/ci.yml#6` round 1 deleted it; there is no
     workflow on this branch right now (see `docs/ARCHITECTURE.md`).
     Until a later round adds it back, the gate only needs to be
     reachable from `./ci.py <name>`.
   - Add a row to `tests/test_gates.py` covering the happy path.
   - If the gate needs a host check, import
     `ghd.platforms` — never `sys.platform`/`os.name`
     inline (see `src/ghd/platforms/README.md`).
3. If you added a hook:
   - `ci/hooks/<name>.py` reading JSON from stdin.
   - Wire in `.claude/settings.json` under the right event.
4. If you changed the Rust workspace shape (a new crate, a new ship
   feature set, a new platform target), update `ci.toml` in the same
   change — it's the checked-in contract a future `ci-lint` validates
   the repo against.
5. Update [README.md](./README.md) if the user-visible workflow changed.

## Architecture

6. Update [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) if crate or
   package responsibilities changed.

## Release flow

7. Update [docs/RELEASE.md](./docs/RELEASE.md) if build or publish
   behavior changed.
8. If the composite action's surface changed, update `action.yml` (and
   `action/cleanup/action.yml` if the cleanup contract changed), and
   re-run `./ci.py action_yaml action_surface` locally to confirm the
   gates still pass.

## Agent guidance

9. Update [CLAUDE.md](./CLAUDE.md) if agent-facing rules changed (the
   essential-rules list, hooks vs gates split, etc.).
10. Update [LINTING.md](./LINTING.md) if the lint surface changed.

## Versioning

11. For version bumps, keep Python (`pyproject.toml::project.version`)
    and Rust (`Cargo.toml::workspace.package.version`) aligned. The
    release pipeline assumes they match.

## Dropping the requirement

If a step on this list stops applying (e.g., gates aren't keyed by
GATE_ORDER anymore), update this file too. An update procedure that
references removed concepts is worse than no procedure.
