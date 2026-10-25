#!/usr/bin/env python3
"""PostToolUse hook: every directory must have a README.md of >= 50 lines.

After any Edit/Write to a file, this hook checks the containing
directory for a `README.md` and asserts a minimum line count. The
floor exists to force enough prose that a reader actually learns what
the directory is for, not just "Title\\n" placeholders.

Exit codes:
  0 - README.md exists and meets the floor, or check not applicable
  2 - README.md missing or too short (stderr fed back to Claude)

See [zackees/zccache#835 rule 8](https://github.com/zackees/zccache/issues/835)
for the rationale. This is a hook (not a gate) because it fires at the
moment a new directory gets its first file — which is the right
moment to demand the README. A workspace walk would have to know which
directories were newly-populated, which is exactly what the agent
context provides for free.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

EXCLUDED_DIRS = {
    ".git",
    ".github",
    "target",
    ".loop",
    "__pycache__",
    ".venv",
    "node_modules",
    "dist",
    "build",
    ".pytest_cache",
    ".ruff_cache",
    ".mypy_cache",
    ".cache",
}

MIN_LINES = 50


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError:
        return 0

    file_path = data.get("tool_input", {}).get("file_path", "")
    if not file_path:
        return 0

    norm = file_path.replace("\\", "/")
    filename = os.path.basename(norm)
    directory = os.path.dirname(norm)

    # If the file being written IS a README.md, defer the floor check
    # to the next edit in that directory; the agent may still be in the
    # middle of expanding it.
    if filename == "README.md":
        return 0

    parts = Path(norm).parts
    if any(p in EXCLUDED_DIRS for p in parts):
        return 0

    readme = os.path.join(directory, "README.md")
    if not os.path.isfile(readme):
        # Try original (un-normalized) path too.
        orig_dir = os.path.dirname(file_path)
        readme = os.path.join(orig_dir, "README.md") if orig_dir else readme
        if not os.path.isfile(readme):
            print(f"Missing README.md in directory: {directory}", file=sys.stderr)
            print(
                "Every directory must have a README.md (>= 50 lines). Expand it with what's in this directory, why, and the key entry points.",
                file=sys.stderr,
            )
            return 2

    try:
        with open(readme, "rb") as fh:
            line_count = sum(1 for _ in fh)
    except OSError:
        return 0

    if line_count < MIN_LINES:
        print(
            f"README.md in {directory} is {line_count} lines (< {MIN_LINES}). Expand it with what's in this directory, why, and the key entry points.",
            file=sys.stderr,
        )
        return 2

    return 0


if __name__ == "__main__":
    sys.exit(main())
