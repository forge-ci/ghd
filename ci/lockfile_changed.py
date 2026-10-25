"""Did Cargo.lock, uv.lock, or rust-toolchain.toml change vs the parent commit?

zackees/ci.yml#6 round-4B, the `cache-maint` job: `ci-lint cache preprune`
should only spend its (budget-forecasting, entry-deleting) work on a writer
push that actually changed one of these files -- comparing against every
ordinary code-only push would be waste. "Parent commit" is deliberately
simple (`HEAD^`, one commit back on whatever ref this job checked out),
matching the brief; the checkout step needs `fetch-depth: 2` for `HEAD^`
to exist locally.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WATCHED = ("Cargo.lock", "uv.lock", "rust-toolchain.toml")


def main() -> int:
    proc = subprocess.run(
        ["git", "diff", "--name-only", "HEAD^", "HEAD", "--", *WATCHED],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        # No parent commit (a shallow/first commit) or another git error --
        # never crash the job over this; treat conservatively as "changed"
        # so preprune's forecast runs rather than silently skipping it.
        print(
            f"ci/lockfile_changed.py: git diff failed (rc={proc.returncode}): "
            f"{proc.stderr.strip()} -- treating as changed (conservative)",
            file=sys.stderr,
        )
        changed = True
    else:
        changed = bool(proc.stdout.strip())

    print(f"lockfile-changed: {changed} (watched: {', '.join(WATCHED)})")
    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            fh.write(f"changed={'true' if changed else 'false'}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
