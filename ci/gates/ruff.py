"""Python lint + format check via ruff.

Runs `ruff check` (including import-sort, `I`, per `[tool.ruff.lint]`
below -- ci.yml#6 round-4B, GEN-004) and `ruff format --check` over the
Python tree (src, tests, ci, action). Failing format is fixable with
`ruff format`; failing lint points at a real issue (unused imports,
shadowed names, unsorted imports, etc.).

Invoked via `uv run --no-project --with ruff` so we get a hermetic ruff
without triggering the wheel build (soldr PEP 517 backend driving
maturin) the surrounding pyproject.toml
otherwise demands. `--with ruff` provisions the dep at script-time even
when the script itself has no PEP 723 deps declared for it.

Ruff is pinned exactly — `>=0.8` resolved to different patch versions
between local and CI, producing different format-check verdicts. The
pin is the cheapest way to make the gate deterministic.

`--config pyproject.toml` (not `--isolated`): this repo's own
`[tool.ruff]`/`[tool.ruff.lint]` section is the config -- ruff's own
discovery already stops climbing at the first `pyproject.toml` or a
`.git` directory, whichever comes first, so pointing at this repo's file
explicitly is equivalent to normal discovery here and makes the source
of truth visible in this command line instead of implicit.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGETS = ["src", "tests", "ci", "action", "ci.py", "lint", "test", "install"]

# Bump deliberately when you want to adopt a newer ruff. Don't loosen
# this to a range — see the docstring.
RUFF_PIN = "ruff==0.15.18"


def _run(args: list[str]) -> int:
    if shutil.which("uv") is None:
        print("uv not on PATH; cannot run ruff gate", file=sys.stderr)
        return 1
    cmd = [
        "uv",
        "run",
        "--no-project",
        "--with",
        RUFF_PIN,
        "ruff",
        *args,
        "--config",
        str(ROOT / "pyproject.toml"),
    ]
    proc = subprocess.run(cmd, cwd=ROOT, check=False)
    return proc.returncode


def run() -> int:
    rc1 = _run(["check", *TARGETS])
    rc2 = _run(["format", "--check", *TARGETS])
    return rc1 or rc2


if __name__ == "__main__":
    raise SystemExit(run())
