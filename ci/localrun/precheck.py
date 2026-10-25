"""Run `ci_lint precheck --local`: the agent's fast local gate.

zackees/ci.yml#6 section 11, rule 2: "Precheck is also the local gate...
must be runnable by agents, and is never blocked by a tool guard" (the
root cause zccache#1760 names as its second cause). This module is what
`python3 ci/local.py precheck` and `python3 ci/local.py act` (its first
step) both call, and what the PostToolUse/Stop hooks in
`.claude/settings.json` shell out to.
"""

from __future__ import annotations

import os
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from ci.localrun.ci_lint_checkout import CiLintCheckoutError, ensure_ci_lint


@dataclass(frozen=True)
class PrecheckOutcome:
    exit_code: int
    checkout_seconds: float
    run_seconds: float
    was_cold: bool


def run_precheck(
    repo_root: Path, *, title: str = "", stream: bool = True
) -> PrecheckOutcome:
    """Ensure the pinned `ci_lint` checkout, then run `precheck --local`.

    Mirrors CLAUDE.md's documented command exactly:
        PYTHONPATH=<ci.yml checkout> uv run --no-project --with pyyaml \\
            python3 -m ci_lint precheck --repo . --local
    """

    try:
        checkout = ensure_ci_lint(repo_root)
    except CiLintCheckoutError as exc:
        print(f"ci/local.py precheck: {exc}", file=sys.stderr)
        return PrecheckOutcome(
            exit_code=1, checkout_seconds=0.0, run_seconds=0.0, was_cold=False
        )

    cmd = [
        "uv",
        "run",
        "--no-project",
        "--with",
        "pyyaml",
        "python3",
        "-m",
        "ci_lint",
        "precheck",
        "--repo",
        ".",
        "--local",
    ]
    if title:
        cmd += ["--title", title]

    env = dict(os.environ)
    env["PYTHONPATH"] = str(checkout.path)

    start = time.monotonic()
    result = subprocess.run(
        cmd,
        cwd=str(repo_root),
        env=env,
        capture_output=not stream,
        text=True,
        check=False,
    )
    run_seconds = time.monotonic() - start

    if not stream:
        sys.stdout.write(result.stdout or "")
        sys.stderr.write(result.stderr or "")

    print(
        f"[ci/local.py] precheck: checkout {checkout.seconds:.2f}s "
        f"({'cold clone/fetch' if checkout.was_cold else 'warm, no network'}) "
        f"+ run {run_seconds:.2f}s = {checkout.seconds + run_seconds:.2f}s total, "
        f"exit {result.returncode}",
        file=sys.stderr,
    )

    return PrecheckOutcome(
        exit_code=result.returncode,
        checkout_seconds=checkout.seconds,
        run_seconds=run_seconds,
        was_cold=checkout.was_cold,
    )
