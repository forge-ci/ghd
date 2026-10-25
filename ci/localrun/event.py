"""Write act's `--eventpath` JSON through `ci_lint plan --act`.

zackees/ci.yml#6 section 11, rule 7: "The plan drives the event...
replacing bosn's per-repo label adapter." `ci_lint plan --act <path>`
(`ci_lint.plan.build_act_event`) is the one place that JSON is built, so
a local run can never select a lane the real plan wouldn't.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

from ci.localrun.ci_lint_checkout import CheckoutResult


class EventPlanError(RuntimeError):
    pass


def write_act_event(
    repo_root: Path, checkout: CheckoutResult, *, title: str, out_path: Path
) -> Path:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        "uv",
        "run",
        "--no-project",
        "--with",
        "pyyaml",
        "python3",
        "-m",
        "ci_lint",
        "plan",
        "--repo",
        ".",
        "--event-name",
        "pull_request",
        "--title",
        title,
        "--act",
        str(out_path),
    ]
    env = dict(os.environ)
    env["PYTHONPATH"] = str(checkout.path)
    result = subprocess.run(
        cmd,
        cwd=str(repo_root),
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0 or not out_path.is_file():
        print(result.stdout, file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        raise EventPlanError(f"'ci_lint plan --act' failed (exit {result.returncode})")
    return out_path
