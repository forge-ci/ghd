# `tests/integration/`

The `integration` suite declared in `ci.toml`'s `[suites]` table
(`suites.integration = { run = "pytest tests/integration" }`), added by
[zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6) round 3.
Not required (`[suites].integration` has no `required = true`), and not
run by default -- it runs only when a PR title selects it directly
(`[ci-test-integration]`) or transitively (`[ci-full]`, which selects
every suite on every platform).

## Why this exists

Everything under `../` (the `tests/` unit suite) proves the PyO3
extension and the native CLI *each*, individually, do something
sensible. Nothing there proves the two agree with each other -- and they
are built from two entirely separate Cargo targets
(`crates/ghd-py`, `crates/ghd-cli`) that both happen to depend
on the same `template` amalgam. A regression that desyncs one build from
the other (stale cache entry, wrong feature set bundled into one target
but not the other, a cross-target build that silently picked up a
different `template` feature set) would pass every unit test and still
ship a broken package.

`test_installed_end_to_end.py` closes that gap: it calls the Python
binding, the raw PyO3 extension function, and the native CLI (plain and
`--json`) in the same process/host, and asserts their outputs are
byte-identical -- not just "each individually looks right".

## Marker discipline

Every test here carries `pytestmark = pytest.mark.integration`
(registered in `pyproject.toml`'s `[tool.pytest.ini_options] markers`).
This is what keeps the suite optional:

- The `unit` suite's `pytest` invocation (`ci/fast.py python-test`) adds
  `-m "not integration"`, so these tests are collected but always
  deselected there.
- The `integration` suite's own invocation (`ci/fast.py
  integration-test`, `ci/platform_run.py integration-test`) adds
  `-m integration tests/integration` explicitly.

## Where this runs

- **Fast lane**, only when `integration` is in the resolved suite
  selection (`needs.precheck.outputs.fast_suites_json`) -- against the
  editable `uv sync` install, same as the rest of `tests/`.
- **Every platform-run lane**, only when `integration` is in that lane's
  own suite selection (`matrix.lane.suites`, from
  `platform_lanes_todo_json`/`platform_lanes_json`) -- against a *clean*
  venv with the built wheel installed (not the source tree), with
  `pythonpath` explicitly cleared (`-o pythonpath=`) so the test can only
  see the installed package, never `../../src` by accident.

## Conventions

- **No mocks.** Same rule as `../README.md`: exercise the real installed
  artifact (extension + native binary), never a stand-in for either.
- **Every assertion cross-checks two independently-built surfaces**
  against each other, not against a hardcoded expected string -- a
  version bump or a banner-format change should never require editing
  this file.
- Keep this directory's tests fast and few. It is a cross-boundary
  smoke check, not a second copy of `../test_bindings.py`/`test_cli.py`'s
  per-surface coverage.
