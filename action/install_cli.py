#!/usr/bin/env python3
"""Install the `ghd` uv tool for the composite action.

Called from `action.yml`'s "Install ghd via uv tool"
step with the `version` input as `argv[1]` (empty string means
"whatever `uv tool install` resolves by default"). Moved out of an
inline `run: |` shell block with an `if`/`else` so the step stays a
single `run:` line — see zackees/ci.yml#6 round 1: every composite
`run:` step is one line calling a Python script.
"""

from __future__ import annotations

import subprocess
import sys


def main(argv: list[str]) -> int:
    version = argv[1] if len(argv) > 1 else ""
    spec = f"ghd=={version}" if version else "ghd"
    proc = subprocess.run(["uv", "tool", "install", spec], check=False)
    return proc.returncode


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
