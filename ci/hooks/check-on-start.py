#!/usr/bin/env python3
"""SessionStart hook: captures repo state fingerprint.

Saves an MD5 fingerprint of the current `git status --porcelain` so a
matching Stop hook (or follow-up analysis) can detect whether anything
in the worktree changed during the session.

Always exits 0; this hook is observation only, never blocking.
"""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent.parent
SESSION_FINGERPRINT_FILE = PROJECT_ROOT / ".cache" / "session_fingerprint.json"


def _run_cmd(cmd: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        cwd=str(PROJECT_ROOT),
        check=False,
    )


def _current_fingerprint() -> str | None:
    result = _run_cmd(["git", "status", "--porcelain"])
    if result.returncode != 0 or not result.stdout.strip():
        return None
    return hashlib.md5(result.stdout.encode()).hexdigest()


def _save(fingerprint: str) -> None:
    SESSION_FINGERPRINT_FILE.parent.mkdir(parents=True, exist_ok=True)
    SESSION_FINGERPRINT_FILE.write_text(
        json.dumps(
            {
                "fingerprint": fingerprint,
                "description": "Captured at session start",
            }
        )
    )


def main() -> int:
    fingerprint = _current_fingerprint()
    if fingerprint is None:
        if SESSION_FINGERPRINT_FILE.exists():
            SESSION_FINGERPRINT_FILE.unlink()
        return 0
    _save(fingerprint)
    return 0


if __name__ == "__main__":
    sys.exit(main())
