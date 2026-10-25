"""PEP 517 backend smoke: `uv build --wheel` through the soldr backend.

The `[build-system]` in pyproject.toml routes wheel builds through the
soldr backend (which drives a pinned maturin under a rustc-caching
wrapper). `ci/gates/test.py` also exercises the backend now (via `uv
sync`, which installs the project), but that's a full-project-sync
path; this gate specifically checks the `uv build --wheel` entry point
a downstream `pip install ghd` (sdist build) or a
release job would use, without installing anything.

Linux-only: soldr's compile daemon cannot spawn on GHA macOS/Windows
runners (zackees/soldr#1300) — macOS fails with "embedded compile
dispatch failed after 30000ms budget: NotRunning" and Windows wedges
the step for ~an hour. On those platforms the gate skips (returns 0)
rather than burning an hour to report a known-upstream condition.
Re-enable everywhere once soldr#1300 is fixed.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Host-branching lives only in the `platform-code` allowlist paths
# (ci.toml's `[allow] platform-code`). Importing this is cheap: it has
# zero runtime dependencies and never needs the `_native` extension
# built. See that module's docstring.
sys.path.insert(0, str(ROOT / "src"))
# pylint: disable-next=wrong-import-position
from ghd.platforms import is_linux  # noqa: E402


def run() -> int:
    if not is_linux():
        print(
            "backend_smoke: skipped on non-Linux (soldr compile daemon "
            "cannot spawn on GHA macOS/Windows runners — "
            "zackees/soldr#1300). The backend path is exercised on the "
            "Linux lanes."
        )
        return 0
    if shutil.which("uv") is None:
        print("uv not on PATH; cannot run backend_smoke gate", file=sys.stderr)
        return 1
    proc = subprocess.run(
        ["uv", "build", "--wheel", "--out-dir", "dist/backend-smoke"],
        cwd=ROOT,
        check=False,
    )
    return proc.returncode


if __name__ == "__main__":
    raise SystemExit(run())
