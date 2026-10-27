"""Python static analysis via pylint.

zackees/ci.yml#6 round-4B (GEN-004): the fast lane runs `pylint` over
`src/`, `ci/`, `tests/`, `action/` -- a different signal than ruff's
(ruff catches unused imports/shadowing/unsorted imports fast; pylint
adds cross-module checks ruff doesn't do: unresolved attribute access,
too-many-locals/branches/statements complexity, duplicate code). Its
config lives in `pyproject.toml`'s `[tool.pylint.*]` tables (minimal,
itemized -- see that file's comment for what's disabled and why; never a
blanket `disable = "all"`).

Invoked via `uv run --no-project --with pylint`, same reasoning as
`ci/gates/ruff.py`: avoid triggering the maturin/soldr wheel build a
bare `uv run` would demand from the root `pyproject.toml`.

Requires the built `_native` extension to be importable (needed for
`extension-pkg-allow-list` to actually introspect it rather than merely
declare it, and to avoid `tests/` import-time failures) -- run this gate
after the `build` gate, same ordering `ci.py`'s `GATE_ORDER` already
uses for `ruff`.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGETS = ["agent-plugin/bootstrap.py", "src", "ci", "tests", "action"]

# Bump deliberately when you want to adopt a newer pylint -- same
# determinism rationale as ruff.py's RUFF_PIN. `pytest` is added to the
# same ephemeral env (unpinned: only needed so pylint can resolve `import
# pytest` in tests/*.py without an E0401 -- this gate never executes a
# test) -- pylint's own `--with pylint==4.0.9` env has no test framework
# in it otherwise.
PYLINT_PIN = "pylint==4.0.9"


def run() -> int:
    if shutil.which("uv") is None:
        print("uv not on PATH; cannot run pylint gate", file=sys.stderr)
        return 1
    cmd = [
        "uv",
        "run",
        "--no-project",
        "--with",
        PYLINT_PIN,
        "--with",
        "pytest",
        "python3",
        "-m",
        "pylint",
        *TARGETS,
        # Passed explicitly (duplicating pyproject.toml's
        # `[tool.pylint.main]` table): observed empirically that a
        # multi-target invocation (this one -- `src ci tests action`
        # together) does not reliably apply the TOML-declared
        # `extension-pkg-allow-list` to every file in the same astroid
        # session, even though a single-file invocation of the one file
        # that needs it (tests/integration/test_installed_end_to_end.py)
        # does. The CLI flag is unconditional and file-order-independent.
        "--extension-pkg-allow-list=ghd._native",
    ]
    print(f"+ {' '.join(cmd)}", flush=True)
    # Same reasoning as ci/fast.py's `_run_isolated_soldr`: this nested
    # `uv run --no-project` must resolve its OWN nested ephemeral
    # environment, not inherit an outer `uv run --no-project --script
    # ci.py`'s `VIRTUAL_ENV` (set when this gate runs via `./ci.py
    # pylint`) -- observed empirically (round-4B worker report) as
    # nondeterministic `E0401: Unable to import 'pytest'` findings that
    # vanished when invoked directly (no outer uv env active). Stripping
    # `VIRTUAL_ENV`/`UV_*` makes the nested resolution independent of
    # whatever uv-managed env is already active in this process.
    env = {
        k: v
        for k, v in os.environ.items()
        if k != "VIRTUAL_ENV" and not k.startswith("UV_")
    }
    proc = subprocess.run(cmd, cwd=ROOT, env=env, check=False)
    return proc.returncode


if __name__ == "__main__":
    raise SystemExit(run())
