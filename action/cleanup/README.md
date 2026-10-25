# `action/cleanup/`

Post-job composite step that pairs with the root `action.yml`. Removes
the `ghd` uv tool install and trims the uv cache
so the runner leaves the same shape it had before this action ran.

Pin it explicitly with:

```yaml
- uses: forge-ci/ghd/action/cleanup@v1
  if: always()
```

The `if: always()` matters — you want cleanup to run even if an
earlier step failed, otherwise you leak the tool install into the
next job's cache restore.

## Why this is split out (not folded into the main action's post step)

Composite actions in GitHub Actions support a single `runs.using:
composite` with a list of steps. There's no built-in `post:` hook the
way JavaScript actions have. Consumers who want cleanup behavior have
to opt in explicitly by adding the cleanup step themselves. Splitting
it into a sibling composite action makes the opt-in mechanical: one
line, no copy-paste of the cleanup logic itself.

## What it actually does

1. `uv tool uninstall ghd` (no-op if not installed).
2. `uv cache prune --ci` to drop entries the next job won't reuse.

Both live in `cleanup.py` (this directory), invoked as `python3
${{ github.action_path }}/cleanup.py uninstall` / `... prune-cache` —
one line per step, no inline shell. Both subcommands swallow their own
failures and always return 0, matching the previous
`... 2>/dev/null || true` behavior without using shell control flow
(`||`) inside a `run:` line — see
[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) round 1.

## Surface contract

Validated by `ci/gates/action_yaml.py` alongside the root `action.yml`.
Both files must satisfy:

- top-level `name`, `description`, `runs` present;
- `runs.using == composite`;
- every input has a `description`;
- every `run:` step declares `shell:`;
- `runs.steps` is a non-empty list.

A break in any of these fails the `action_yaml` gate, and the PR check
surface points at exactly which file went wrong.
