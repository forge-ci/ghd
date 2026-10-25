#!/usr/bin/env python3
"""PostToolUse/Stop hook: run `ci/local.py precheck` on contract-shaped edits.

zackees/ci.yml#6 section 11's zccache#1760 fixture: the incident's root
cause 1 was "no local gate runs [the guard]... not in the PostToolUse
hooks... not in the Stop hook". This hook closes that gap for this
repository specifically for the files a contract violation can hide in:
any workflow or composite action under `.github/**`, `ci.toml` itself,
and the three manifests a cache-shape or tool-shape violation can ride
in on (`Cargo.toml`, `pyproject.toml`, `bosn.toml`) plus any
`action.yml`.

Two invocations, one script:
  - PostToolUse (`Edit|Write|MultiEdit`, no `--stop`): reads
    `tool_input.file_path` from stdin and only runs precheck when it
    matches one of the paths above -- everything else is a fast no-op.
  - Stop (`--stop`): always runs precheck, regardless of which tool
    touched what. This is the unconditional backstop -- it also catches
    a contract-shaping edit made through Bash (`git apply`, a generated
    file, etc.) that never went through Edit/Write/MultiEdit at all.

`ci/local.py precheck` is intentionally never blocked by
`ci/hooks/tool_guard.py` (it is a `python3` invocation of a named
script, which that hook always allows) -- zccache#1760's root cause 2
was the local gate itself being unreachable from inside an agent
session.

Exit codes:
  0 - not applicable, or precheck passed
  2 - precheck failed (stderr fed back to Claude, with the findings)
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent

WATCHED_DIR_PREFIXES = (".github/",)
WATCHED_EXACT_PATHS = {"ci.toml", "bosn.toml"}
WATCHED_BASENAMES = {"Cargo.toml", "pyproject.toml", "action.yml"}


def _matches(norm_path: str) -> bool:
    if norm_path in WATCHED_EXACT_PATHS:
        return True
    if any(norm_path.startswith(prefix) for prefix in WATCHED_DIR_PREFIXES):
        return True
    basename = norm_path.rsplit("/", 1)[-1]
    return basename in WATCHED_BASENAMES


def _edited_path_from_stdin() -> str | None:
    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError:
        return None
    file_path = data.get("tool_input", {}).get("file_path", "")
    if not file_path:
        return None
    return file_path.replace("\\", "/")


def _run_precheck() -> int:
    result = subprocess.run(
        [sys.executable, "ci/local.py", "precheck"],
        cwd=str(REPO_ROOT),
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stdout)
        sys.stderr.write(result.stderr)
        print(
            "\nlocal_precheck_guard: 'python3 ci/local.py precheck' failed -- fix every "
            "VIOLATION above (see each rule's 'fix:' line) before continuing. "
            "Re-run 'python3 ci/local.py precheck' directly to iterate.",
            file=sys.stderr,
        )
        return 2
    return 0


def main() -> int:
    if "--stop" in sys.argv[1:]:
        return _run_precheck()

    file_path = _edited_path_from_stdin()
    if file_path is None:
        return 0
    norm = file_path
    repo_root_str = str(REPO_ROOT).replace("\\", "/")
    if norm.startswith(repo_root_str + "/"):
        norm = norm[len(repo_root_str) + 1 :]
    if not _matches(norm):
        return 0
    return _run_precheck()


if __name__ == "__main__":
    sys.exit(main())
