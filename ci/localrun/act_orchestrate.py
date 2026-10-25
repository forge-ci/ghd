"""Run the original PR workflow through Bosn's supervised, pinned act2.

The default calls the pinned shared ci-lint gate, which submits the complete
workflow, including precheck, build/test/wheel checks, Dylint and CI OK,
then validates execution evidence before writing commit trailers. Explicit lane selection is diagnostic
and never represents whole-workflow coverage. Bosn owns event construction,
frozen checkout, isolated engines, cache storage, logs and cleanup.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from ci.localrun.ci_lint_checkout import pinned_sha
from ci.localrun.precheck import run_precheck

WORKFLOW = ".github/workflows/ci.yml"


def run_act(repo_root: Path, *, lanes_arg: str | None, title: str) -> int:
    outcome = run_precheck(repo_root, title=title)
    if outcome.exit_code != 0:
        print("[ci/local.py] stopped at precheck", file=sys.stderr)
        return outcome.exit_code

    if lanes_arg is None and not title:
        # The shared tool owns reuse, execution proof and commit trailers.
        return subprocess.run(
            [
                "uvx",
                "--from",
                f"git+https://github.com/zackees/ci.yml@{pinned_sha(repo_root)}",
                "ci-lint",
                "local-gate",
                "run",
                "--repo",
                str(repo_root),
            ],
            cwd=repo_root,
            check=False,
        ).returncode

    jobs: list[str | None] = [None]
    if lanes_arg is not None:
        selected = [value.strip() for value in lanes_arg.split(",")]
        if any(not value for value in selected) or len(set(selected)) != len(selected):
            print(
                "[ci/local.py] lanes must be distinct nonempty job IDs", file=sys.stderr
            )
            return 1
        jobs = []
        jobs.extend(selected)
        print(
            "[ci/local.py] selected-job diagnostics; complete PR coverage requires the default run",
            file=sys.stderr,
        )

    for job in jobs:
        command = [
            "bosn",
            "ci",
            "run",
            "--workspace",
            str(repo_root),
            "--workflow",
            WORKFLOW,
            "--trigger",
            "pr",
            "--wait",
        ]
        if title:
            command.extend(["--pr-title", title])
        if job is not None:
            command.extend(["--job", job])
        print(
            f"[ci/local.py] Bosn → act2: {job or 'complete PR workflow'}",
            file=sys.stderr,
        )
        try:
            result = subprocess.run(command, cwd=repo_root, check=False)
        except OSError as error:
            print(f"[ci/local.py] cannot start Bosn: {error}", file=sys.stderr)
            return 1
        if result.returncode != 0:
            return result.returncode
    return 0
