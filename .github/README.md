# `.github/`

GitHub-specific metadata. Kept deliberately thin per
[zackees/zccache#835 rule 6](https://github.com/zackees/zccache/issues/835):
every CI step is a single line — `run: ./ci.py <gate>` or `run:
python3 ci/<script>.py ...` — never multi-line shell. The actual logic
lives in `ci/gates/*.py` and `ci/{fast,dylint,ci_ok}.py` so the same
bytes run on a developer laptop.

## Layout

```
.github/
├── actions/
│   └── soldr/           # the ONLY `zackees/setup-soldr` call site
├── workflows/
│   ├── ci.yml            # the only entrypoint
│   ├── ci-pre.yml   # workflow_call, gates ci.yml
│   └── README.md
└── README.md             # this file
```

## The workflows

[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) round 1
deleted the previous `.github/workflows/ci.yml` here (an 8-entry native
matrix with bare `cargo`/`maturin` calls, no `setup-soldr`, and a retired
`macos-13` runner that queued 24h on every run — 0 of 27 historical runs
ever succeeded). Round 2 added it back, this time generated to match
`ci.toml`'s `[platforms]`/`[suites]`/`[flow.*]` declarations and gated by
`ci-pre.yml` (`workflow_call` only, ≤ 30 s, no tool installs). See
`.github/workflows/README.md` for the job graph and step-shape rules.

- `ci.toml` at the repo root is the exact platform/suite/tag/cache
  contract those two workflow files implement — nothing is hand-written
  into YAML that `ci.toml` already declares.
- `./ci.py all` locally runs the same gate set the `fast` job's fmt/
  clippy steps call.
- `python3 -m ci_lint precheck --repo . --local` is the same check
  `ci-pre.yml` runs, and this repo's agent Stop-hook / pre-push gate.

## The composite action

`.github/actions/soldr/` wraps `zackees/setup-soldr` with `ci.toml`'s
three mandatory inputs baked in
(`solo-toolchain-cache: false`, `cook-delta: false`, `save-cache: auto`)
so no lane can silently reenable a retired cache family
(zccache#1760, `CACHE-009`). It is the ONLY place `zackees/setup-soldr@…`
may appear in this repository — `ci-lint` precheck rejects a second call
site. See its own README before adding a lane that needs a new
setup-soldr input.

## Why this folder stays intentionally small

The historic anti-pattern is multi-line shell embedded in YAML —
unlintable, untestable, only validates when CI runs. Pushing logic into
`ci/gates/<name>.py` (and, for CI-specific orchestration, `ci/fast.py` /
`ci/dylint.py` / `ci/ci_ok.py`) makes each step testable from a plain
Python invocation, not just a live workflow run. The workflow files only
have to know which gates/suites/lanes exist, never what they do.

## Adding a new gate or lane

1. Gate (runs on every `./ci.py all` too): write `ci/gates/<name>.py`
   with `def run() -> int`, using `soldr` for any Rust/wheel command, and
   register it in `ci.py::GATE_ORDER`.
2. CI-only orchestration (a new `fast`/`dylint` step, not a developer
   entry point): add a subcommand to the relevant `ci/*.py` module and
   one new `run:` line in `ci.yml`.
3. A genuinely new lane (platform, suite, tag): declare it in `ci.toml`
   first — the workflow reads the plan, it never invents selection logic
   of its own.

Never add a third file to `.github/workflows/` — see that directory's
own README.
