"""`soldr cargo check --workspace` gate.

The cheapest gate that proves the Rust workspace still compiles. We use
`check` instead of `build` because the downstream `test` gate already
does a real build, so paying for two builds is wasted work. `--locked`
refuses to update `Cargo.lock`.

**Fatal in `./ci.py all`.** If this gate fails, every later gate
(`test`, `action_surface`, anything that touches the compiled binaries)
will produce noise instead of signal. `ci.py` sees `build` in
`FATAL_GATES` and halts the run, reporting only the build failure in
the final summary. See [zccache#835 rule 7](https://github.com/zackees/zccache/issues/835).

Goes through `soldr` rather than bare `cargo` — see zackees/ci.yml#6
round 1 (`RUST-001`).
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def run() -> int:
    if shutil.which("soldr") is None:
        print("soldr not on PATH; cannot run build gate", file=sys.stderr)
        return 1
    proc = subprocess.run(
        ["soldr", "cargo", "check", "--workspace", "--all-targets", "--locked"],
        cwd=ROOT,
        check=False,
    )
    return proc.returncode


if __name__ == "__main__":
    raise SystemExit(run())
