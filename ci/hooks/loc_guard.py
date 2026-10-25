#!/usr/bin/env python3
"""PostToolUse hook: enforces per-file LOC budget on source edits.

After any Edit/Write to a source file, counts the resulting line count
and:
  - emits a warning (exit 0 + stderr) when LOC > WARN_THRESHOLD
  - hard-errors (exit 2 + stderr) when LOC > ERROR_THRESHOLD

This is the per-edit half of the LOC story; the per-CI half lives in
`ci/gates/loc.py`. Together they catch growth from agent edits (hook)
and from manual pushes / rebases (gate).

Exit codes:
  0 - file is fine, missing, not a tracked source extension, or only warned
  2 - file exceeds ERROR_THRESHOLD (stderr fed back to Claude as a block)
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

WARN_THRESHOLD = 1000
ERROR_THRESHOLD = 1500

SOURCE_EXTS = {
    ".rs",
    ".py",
    ".ts",
    ".tsx",
    ".js",
    ".jsx",
    ".go",
    ".java",
    ".kt",
    ".swift",
    ".c",
    ".cc",
    ".cpp",
    ".cxx",
    ".h",
    ".hh",
    ".hpp",
}

EXCLUDED_DIRS = {
    ".git",
    "target",
    ".cargo",
    ".rustup",
    ".venv",
    "node_modules",
    "__pycache__",
    "dist",
    "build",
    ".claude",
    ".pytest_cache",
    ".ruff_cache",
    ".mypy_cache",
    ".cache",
}

REFRAIN = "Convention: `foo.rs` -> `foo/mod.rs` + per-domain submodules, with `pub use` re-exports in `mod.rs` so the public path is unchanged."


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError:
        return 0

    file_path = data.get("tool_input", {}).get("file_path", "")
    if not file_path:
        return 0

    norm = file_path.replace("\\", "/")
    ext = os.path.splitext(norm)[1].lower()
    if ext not in SOURCE_EXTS:
        return 0

    parts = Path(norm).parts
    if any(p in EXCLUDED_DIRS for p in parts):
        return 0

    if not os.path.isfile(file_path):
        return 0

    try:
        with open(file_path, "rb") as fh:
            loc = sum(1 for _ in fh)
    except OSError:
        return 0

    if loc > ERROR_THRESHOLD:
        print(
            f"LOC guard: {file_path} is {loc} lines (> {ERROR_THRESHOLD}). "
            f"Split into focused submodules before continuing. "
            f"Refactor target: < {WARN_THRESHOLD} lines so future edits have "
            f"headroom. {REFRAIN}",
            file=sys.stderr,
        )
        return 2

    if loc > WARN_THRESHOLD:
        print(
            f"LOC guard warning: {file_path} is {loc} lines "
            f"(> {WARN_THRESHOLD}). Refactor down to < {WARN_THRESHOLD} so "
            f"future edits can land without crossing {ERROR_THRESHOLD} and "
            f"hard-blocking. {REFRAIN}",
            file=sys.stderr,
        )

    return 0


if __name__ == "__main__":
    sys.exit(main())
