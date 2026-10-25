"""Exact-SHA release guard (zackees/ci.yml#6 §3, ci.yml#4, round 5
deliverable 1): on `workflow_dispatch`, the release graph MUST run
against exactly `inputs.sha`, and that commit MUST already be reachable
from `main` -- refusing to build an unreviewed/unmerged commit as a
"release". This is the `release-guard` job's ONE `run:` line
(CLAUDE.md rule 6 / `GEN-005`); it must fail BEFORE any build step in
every downstream release job (`needs: [precheck, release-guard]`).

For a `[release]` PR rehearsal (or any other non-dispatch event that
still selects the release flow, e.g. `nightly`), there is no
`inputs.sha` to pin against -- the guard is a documented no-op so the
SAME job id can sit ahead of every release-scoped job regardless of
which event selected the release flow (`ci.toml` `[tags].release` /
`[flow.nightly]`).

Never invokes `soldr`/`cargo`/`uv` -- this only inspects the checkout's
own git history (`git` is an `[allow].tools` entry, never a banned bare
tool -- `TOOL-001`'s banned list is cargo/rustc/rustup/maturin/cross/
cibuildwheel/pip/pipx/twine/curl/wget, not git).
"""

from __future__ import annotations

import os
import subprocess
import sys

_HEX_DIGITS = set("0123456789abcdefABCDEF")


def _is_40_hex(value: str) -> bool:
    return len(value) == 40 and all(c in _HEX_DIGITS for c in value)


def _git(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], check=False, capture_output=True, text=True)


def main() -> int:
    event_name = os.environ.get("RELEASE_GUARD_EVENT", "")
    dispatch_sha = os.environ.get("RELEASE_GUARD_SHA", "")
    main_ref = os.environ.get("RELEASE_GUARD_MAIN_REF", "origin/main")

    if event_name != "workflow_dispatch":
        print(
            f"release_guard: event={event_name!r} (not workflow_dispatch) "
            "-- no inputs.sha to pin against, guard is a no-op"
        )
        return 0

    if not dispatch_sha or not _is_40_hex(dispatch_sha):
        print(
            f"release_guard: inputs.sha {dispatch_sha!r} is not a 40-hex commit SHA",
            file=sys.stderr,
        )
        return 1

    head = _git("rev-parse", "HEAD")
    if head.returncode != 0:
        print(
            f"release_guard: git rev-parse HEAD failed: {head.stderr.strip()}",
            file=sys.stderr,
        )
        return 1
    head_sha = head.stdout.strip()

    if head_sha.lower() != dispatch_sha.lower():
        print(
            f"release_guard: checked-out HEAD {head_sha} != inputs.sha {dispatch_sha} "
            "-- the release-guard job's checkout step must use "
            "ref: ${{ github.event.inputs.sha }}",
            file=sys.stderr,
        )
        return 1

    ancestor = _git("merge-base", "--is-ancestor", dispatch_sha, main_ref)
    if ancestor.returncode != 0:
        detail = ancestor.stderr.strip() or f"exit {ancestor.returncode}"
        print(
            f"release_guard: {dispatch_sha} is NOT reachable from {main_ref} ({detail}) "
            "-- refusing to run the release graph against a commit that is not on main. "
            "Merge the candidate commit into main first, then dispatch again with its "
            "real merge SHA.",
            file=sys.stderr,
        )
        return 1

    print(
        f"release_guard: PASS -- {dispatch_sha} is HEAD and reachable from {main_ref}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
