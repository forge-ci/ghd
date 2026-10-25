#!/usr/bin/env python3
"""Resolve the installed `ghd` binary and export it.

Called from `action.yml`'s "Expose binary path" step. Writes the
`binary-path` output and prepends the binary's directory to `PATH` via
`$GITHUB_OUTPUT` / `$GITHUB_PATH`, replacing an inline `run: |` shell
block that computed `dirname "$(command -v ghd)"` — see
zackees/ci.yml#6 round 1: every composite `run:` step is one line
calling a Python script.
"""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path


def main() -> int:
    binary = shutil.which("ghd")
    if binary is None:
        print(
            "ghd not found on PATH after `uv tool install`",
            file=sys.stderr,
        )
        return 1

    binary_path = str(Path(binary).resolve())
    github_output = os.environ.get("GITHUB_OUTPUT")
    github_path = os.environ.get("GITHUB_PATH")

    if github_output:
        with open(github_output, "a", encoding="utf-8") as fh:
            fh.write(f"binary-path={binary_path}\n")
    if github_path:
        with open(github_path, "a", encoding="utf-8") as fh:
            fh.write(f"{Path(binary_path).parent}\n")

    print(f"ghd resolved at {binary_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
