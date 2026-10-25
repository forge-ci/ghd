"""Static parse + structural check of the composite-action contract.

The repo ships a top-level `action.yml` (used when consumers do
`uses: forge-ci/ghd@v1`) and an `action/cleanup/
action.yml` for the post-run step. This gate parses both files with
PyYAML and asserts:

  - Required top-level keys (`name`, `description`, `runs`) present.
  - `runs.using` is `composite` (the contract this template targets).
  - Every input declared in `inputs:` has a `description`.
  - `runs.steps` is a non-empty list and every step has either `uses`
    or `run`.

That's enough to catch the silly failure modes (typo in a key, missing
description) without paying for a real `uses: ./` end-to-end run. The
runtime check that subcommands referenced by the shell snippets exist
in the built binary lives in `action_surface.py` — see [zccache#835
rule 10](https://github.com/zackees/zccache/issues/835).
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

# `yaml` (PyYAML) is the only third-party dep this gate needs. It's
# declared as a PEP 723 inline dep on `ci.py`, so `./ci.py action_yaml`
# always has it. The try/except keeps the module importable in the
# pytest venv (which doesn't install pyyaml) so the contract test in
# `tests/test_gates.py` can introspect `run()`'s signature; `run()`
# fails fast with a clear message if pyyaml is unavailable at call time.
try:
    import yaml  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover
    yaml = None  # type: ignore[assignment]

ROOT = Path(__file__).resolve().parents[2]

ACTION_FILES = [
    ROOT / "action.yml",
    ROOT / "action" / "cleanup" / "action.yml",
]

REQUIRED_TOP = ("name", "description", "runs")


def _check_one(path: Path) -> list[str]:  # noqa: C901
    assert yaml is not None  # guarded by run(); narrow for type-checkers
    errs: list[str] = []
    if not path.is_file():
        return [f"{path.relative_to(ROOT)}: missing"]

    try:
        data: Any = yaml.safe_load(path.read_text(encoding="utf-8"))
    except yaml.YAMLError as exc:
        return [f"{path.relative_to(ROOT)}: invalid YAML: {exc}"]

    if not isinstance(data, dict):
        return [f"{path.relative_to(ROOT)}: top level must be a mapping"]

    for key in REQUIRED_TOP:
        if key not in data:
            errs.append(f"{path.relative_to(ROOT)}: missing top-level `{key}`")

    runs = data.get("runs")
    if isinstance(runs, dict):
        using = runs.get("using")
        if using != "composite":
            errs.append(
                f"{path.relative_to(ROOT)}: runs.using must be 'composite' (got {using!r})"
            )
        steps = runs.get("steps")
        if not isinstance(steps, list) or not steps:
            errs.append(
                f"{path.relative_to(ROOT)}: runs.steps must be a non-empty list"
            )
        else:
            for i, step in enumerate(steps):
                if not isinstance(step, dict):
                    errs.append(
                        f"{path.relative_to(ROOT)}: runs.steps[{i}] not a mapping"
                    )
                    continue
                if "uses" not in step and "run" not in step:
                    errs.append(
                        f"{path.relative_to(ROOT)}: runs.steps[{i}] must have `uses` or `run`"
                    )
                # Composite shell steps must declare `shell:`.
                if "run" in step and "shell" not in step:
                    errs.append(
                        f"{path.relative_to(ROOT)}: runs.steps[{i}] is a `run:` step but is missing `shell:`"
                    )

    inputs = data.get("inputs")
    if isinstance(inputs, dict):
        for name, spec in inputs.items():
            if not isinstance(spec, dict):
                errs.append(f"{path.relative_to(ROOT)}: inputs.{name} not a mapping")
                continue
            if not spec.get("description"):
                errs.append(
                    f"{path.relative_to(ROOT)}: inputs.{name} missing description"
                )

    return errs


def run() -> int:
    if yaml is None:
        print(
            "action_yaml: PyYAML not available. Run via `./ci.py action_yaml` so the PEP 723 inline-deps path provides it.",
            file=sys.stderr,
        )
        return 1
    all_errs: list[str] = []
    for path in ACTION_FILES:
        all_errs.extend(_check_one(path))
    if all_errs:
        for e in all_errs:
            print(f"action-yaml: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(run())
