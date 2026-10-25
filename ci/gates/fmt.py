"""`soldr cargo fmt --check` gate.

Runs `soldr cargo fmt --all -- --check` against the Rust workspace. Does
NOT write changes; failing is the signal to run `soldr cargo fmt --all`
locally.

This gate is fast (no compilation, no target/ writes), so it runs before
the heavier clippy / build gates in `ci.py::GATE_ORDER`. Goes through
`soldr` rather than bare `cargo` — see zackees/ci.yml#6 round 1 (`RUST-001`:
compile-bearing/toolchain-bearing Rust CI must not bypass Soldr) — which
also resolves the pinned `rust-toolchain.toml` channel instead of
whatever `cargo` happens to be first on PATH.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def run() -> int:
    if shutil.which("soldr") is None:
        print("soldr not on PATH; cannot run fmt gate", file=sys.stderr)
        return 1
    proc = subprocess.run(
        ["soldr", "cargo", "fmt", "--all", "--", "--check"],
        cwd=ROOT,
        check=False,
    )
    return proc.returncode


if __name__ == "__main__":
    raise SystemExit(run())
