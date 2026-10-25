#!/usr/bin/env python3
"""Smoke-test the installed `ghd` surface.

Called from `action.yml`'s "Smoke-test the surface" step. Runs
`ghd --version` and `ghd --help` (both need PATH set
up by the preceding "Expose binary path" step) and fails the action if
either exits non-zero. Replaces an inline two-line `run: |` shell block
— see zackees/ci.yml#6 round 1: every composite `run:` step is one line
calling a Python script.
"""

from __future__ import annotations

import subprocess


def main() -> int:
    for args in (["--version"], ["--help"]):
        proc = subprocess.run(["ghd", *args], check=False)
        if proc.returncode != 0:
            return proc.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
