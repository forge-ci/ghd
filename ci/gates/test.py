"""Test gate: `soldr cargo test --workspace --locked` + wheel-installed pytest.

Two steps:

1. `soldr cargo test --workspace --locked` — every declared Rust test
   binary (`ci.toml`'s `[rust.tests].binaries`): the `ghd-core`,
   `ghd-platform`, and `ghd-daemon` unit harnesses, plus the
   `template:test:api` and `ghd-cli:test:cli` integration targets.
2. `uv sync --frozen` — installs the project itself through the real PEP
   517 backend (`build-backend = "soldr"`), which builds the `_native`
   extension AND bundles `ghd-cli` into the venv's `Scripts`/`bin`
   via `[tool.soldr.pep517] bundle-bins` (see pyproject.toml). Then
   `pytest` runs against that installed package — the same artifact
   shape a real `pip install` produces, not an in-place `cargo build`
   the test harness reads around.

Deliberately does NOT call `maturin develop` directly (that was this
gate's previous shape). `ci/gates/*.py` must never invoke bare `cargo` or
`maturin` — see zackees/ci.yml#6 round 1 (`RUST-001`/`PKG-003`). Reserve
this opt-in to a full project sync for named entry points — see
[zccache#835 rule 5](https://github.com/zackees/zccache/issues/835).
Other gates use `./ci.py`'s `--no-project --script` discipline so they
don't pay that cost.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def _run(cmd: list[str]) -> int:
    proc = subprocess.run(cmd, cwd=ROOT, check=False)
    return proc.returncode


def run() -> int:
    if shutil.which("soldr") is None:
        print("soldr not on PATH; cannot run test gate", file=sys.stderr)
        return 1
    if shutil.which("uv") is None:
        print("uv not on PATH; cannot run test gate", file=sys.stderr)
        return 1

    rc = _run(["soldr", "cargo", "test", "--workspace", "--locked"])
    if rc != 0:
        return rc

    # Installs the project itself via the soldr PEP 517 backend: builds
    # `_native` and bundles `ghd-cli`, then `uv` installs the wheel
    # into .venv. This is the real packaging path, not a bare `maturin
    # develop` shortcut.
    rc = _run(["uv", "sync", "--frozen"])
    if rc != 0:
        return rc

    return _run(["uv", "run", "--no-sync", "pytest"])


if __name__ == "__main__":
    raise SystemExit(run())
