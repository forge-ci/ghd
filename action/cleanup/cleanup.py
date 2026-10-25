#!/usr/bin/env python3
"""Post-job cleanup for the `ghd` composite action.

Usage:
    python3 action/cleanup/cleanup.py uninstall
    python3 action/cleanup/cleanup.py prune-cache

Both subcommands are best-effort: a failure (tool not installed, cache
already empty) is swallowed and the exit code is always 0, matching the
previous inline `... 2>/dev/null || true` shell steps. Moved to a
script so those steps stay a single `run:` line with no shell control
flow (`||`) — see zackees/ci.yml#6 round 1.
"""

from __future__ import annotations

import subprocess
import sys


def uninstall() -> int:
    subprocess.run(["uv", "tool", "uninstall", "ghd"], check=False)
    return 0


def prune_cache() -> int:
    subprocess.run(["uv", "cache", "prune", "--ci"], check=False)
    return 0


COMMANDS = {"uninstall": uninstall, "prune-cache": prune_cache}


def main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[1] not in COMMANDS:
        print(f"usage: {argv[0]} {{{'|'.join(COMMANDS)}}}", file=sys.stderr)
        return 2
    return COMMANDS[argv[1]]()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
