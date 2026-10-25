"""GEN-021 default-branch verified reuse: the `reuse-decision` job's whole
body (zackees/ci.yml#162).

A push to `main` whose one merged PR already validated a byte-identical
tree may skip the `fast` lane. No merge queue is configured for this
repository, so without this the post-merge run is the only validation of
what actually landed.

The decision itself lives in `ci_lint reuse-check`, which is fail-closed
and says nothing unless all of these hold:

  1. exactly one PR is associated with the pushed SHA and is merged into
     the default branch with `merge_commit_sha` equal to that SHA,
  2. the pushed commit's tree equals that PR head's tree,
  3. the newest decisive `pull_request` run of this workflow on that head
     concluded `success`, and
  4. every `--required-job` display name in it is `success` and completed
     within `--max-age-hours`.

Only `fast` is proven here, and only `fast` is ever skipped downstream.
`dylint` and the platform lanes are the default-branch cache writers and
keep running on every push: skipping them would stale the caches that PRs
restore from and pay the saving back many times over.

This script exists to keep the workflow's `run:` a single line (GEN-005,
the Python-delegation shell budget). It is a thin, explicit wrapper -- no
policy of its own lives here.
"""

from __future__ import annotations

import os
import subprocess
import sys

OUT = "reuse-check.json"


def main() -> int:
    digest = os.environ.get("FAST_DIGEST", "").strip()
    if not digest:
        # The plan did not select a `fast` lane, so there is no proving job
        # to name and nothing this decision could ever justify skipping.
        print("FAST_DIGEST is empty: the plan selected no fast lane, no decision made")
        with open(OUT, "w", encoding="utf-8") as fh:
            fh.write('{"schema": 1, "reuse": false, "reason": "no-fast-lane"}\n')
        return 0

    cmd = [
        sys.executable,
        "-m",
        "ci_lint",
        "reuse-check",
        "--workflow",
        "ci.yml",
        "--mode",
        "enforce",
        "--github-output",
        "--out",
        OUT,
        "--required-job",
        f"fast [{digest}]",
    ]
    print(f"+ {' '.join(cmd)}", flush=True)
    # reuse-check exits 0 for every "cannot prove" outcome (reuse=false) and
    # 2 only for a usage error, so a refusal to reuse never fails this job
    # and therefore never blocks the merge.
    return subprocess.run(cmd, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
