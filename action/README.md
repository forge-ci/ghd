# `action/`

GitHub composite-action support files. The top-level `action.yml` lives
at the repo root (that's where `uses: forge-ci/ghd@v1`
looks for it), but secondary actions — most importantly the `cleanup`
post-job step — live here so the root doesn't accumulate.

## Layout

```
action/
├── install_cli.py     # step: install the uv tool (version input handling)
├── expose_binary.py   # step: resolve the binary, set outputs.binary-path + PATH
├── smoke_test.py       # step: --version / --help smoke
└── cleanup/
    ├── action.yml      # post-job uninstall + cache trim
    └── cleanup.py       # both cleanup steps (uninstall / prune-cache)
```

Every `run:` step in both `action.yml` files is a single line —
`python3 ${{ github.action_path }}/<script>.py ...` — never inline
shell logic. `${{ github.action_path }}` matters: a consumer's
`uses: forge-ci/ghd@v1` runs with the *consumer's*
working directory, so a bare relative path like `action/install_cli.py`
would resolve against their checkout, not this action's own files.
`github.action_path` always points at this action's own files
regardless of where it's consumed from. See
[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) round 1
— this replaced multi-line `run: |` shell blocks with `if`/`else` and
`2>/dev/null || true` control flow.

## Why this split

GitHub Actions resolves `uses: <owner>/<repo>@<ref>` to the repo
root's `action.yml`. Any *additional* composite actions consumed by
the main action (via `uses: <owner>/<repo>/action/cleanup@<ref>`,
etc.) need to live in subdirectories with their own `action.yml`.
Putting them under `action/` keeps the repo root focused and matches
the convention you'll find in [`actions/cache`](https://github.com/actions/cache)
and [`actions/setup-node`](https://github.com/actions/setup-node).

## Surface validation

Two gates in `ci/gates/` keep the action contract honest:

- `ci/gates/action_yaml.py` — static parse of every `action.yml`
  under this tree. Asserts `runs.using == composite`, every input has
  a description, every step has either `uses` or `run`, and every
  `run:` step declares `shell:`.

- `ci/gates/action_surface.py` — runtime check that subcommands
  referenced by the action's shell snippets exist in
  `ghd-cli --help`. Cheap — under a second against the built
  binary — and catches the typo class of contract regressions
  without paying for a real `uses: ./` end-to-end build per matrix
  entry.

See [zackees/zccache#835 rule 10](https://github.com/zackees/zccache/issues/835)
for the rationale: the contract that downstream consumers actually
care about is "does the binary's surface match what action.yml's
shell snippets call against?" That's <5 s against the already-built
binary, not minutes per platform.

## Adding a new composite step

1. Create `action/<name>/action.yml` with the composite shape.
2. Reference it from consumers as
   `uses: <owner>/ghd/action/<name>@<ref>`.
3. The `action_yaml` gate will pick it up automatically because it
   walks every `action.yml` under this tree.
