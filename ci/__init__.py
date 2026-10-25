"""CI helper package.

Two structured sub-packages:

  - `ci.gates.*` — workspace-state checks invoked by `./ci.py <gate>`;
    runs on every CI cycle and on developer laptops. Every Rust/wheel
    command here goes through `soldr` (`soldr cargo ...`, `soldr wheel`,
    the soldr PEP 517 backend via `uv build`/`uv sync`) — never bare
    `cargo`/`maturin`. See zackees/ci.yml#6 round 1.
  - `ci.hooks.*` — agent-intent guards wired through `.claude/settings.json`;
    runs only during a Claude/Codex session.

The release-flow helpers this package used to hold (`build_wheel.py`,
`publish.py`) were removed in zackees/ci.yml#6 round 1: the soldr PEP
517 backend now bundles `ghd-cli` into the wheel directly
(`[tool.soldr.pep517] bundle-bins` in `pyproject.toml`), and mock
publish is a later round's work (see `docs/RELEASE.md`). See
`ci/README.md` for the directory map and
[zackees/zccache#835](https://github.com/zackees/zccache/issues/835)
for the gates/hooks design rationale.
"""
