"""`CI OK` gate: the one required check `ci.yml` exposes.

zackees/ci.yml#6 §2/§6. The `ci-ok` job (`if: always()`, `needs:` every
other job) passes the precheck job's `plan`/`reuse_json` outputs,
`toJSON(needs)`, and `toJSON(github.event)` into this script's
environment (never interpolated into a `run:` line -- script-injection
rule, same reason PR titles never are). This is the ONE line that job's
step calls; everything else lives here and in `ci_lint gate`.

`ci_lint gate` decides mergeability: every planned job succeeded AND the
selection itself is mergeable (`plan.json`'s own `mergeable` field is
false for `[no-test]`/`[no-test-<suite>]` regardless of job outcomes --
see ci.toml `[tags]`), where a required job whose result is `skipped`
also counts as success when the plan's `reuse_json` map, live-verified
against the Actions jobs API (`GITHUB_TOKEN`/`actions: read`), proves it
is an identical-digest, identical-head-SHA reuse from an earlier run of
this same PR (zackees/ci.yml#6 round-3A "Reuse verification"). The shared
--attested-workflow consumer separately rechecks local attestations using
the immutable PR base policy; this script does not decide skip eligibility.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _write_if_present(env_name: str, filename: str) -> Path | None:
    """Write `$env_name`'s JSON to `$RUNNER_TEMP/<filename>` and return its
    path, or None if the env var is unset/empty -- both `--reuse` and
    `--event` are optional `ci_lint gate` flags (a push/schedule/dispatch
    run still has a `reuse_json` output, but `github.event` may be small
    or missing a `pull_request` key entirely on those events, which
    `ci_lint gate`'s `--event` parsing already tolerates)."""

    raw = os.environ.get(env_name)
    if not raw:
        return None
    try:
        json.loads(raw)
    except json.JSONDecodeError as exc:
        print(f"ci/ci_ok.py: {env_name} is not valid JSON: {exc}", file=sys.stderr)
        raise SystemExit(2) from exc
    path = Path(os.environ.get("RUNNER_TEMP", "/tmp")) / filename
    path.write_text(raw, encoding="utf-8")
    return path


def main() -> int:
    plan_json = os.environ.get("CI_OK_PLAN_JSON")
    needs_json = os.environ.get("CI_OK_NEEDS_JSON")
    if not plan_json or not needs_json:
        print(
            "ci/ci_ok.py: CI_OK_PLAN_JSON and CI_OK_NEEDS_JSON must be set "
            "(the ci-ok job's step env:, never interpolated into run:)",
            file=sys.stderr,
        )
        return 2

    # Validate before writing -- a malformed upstream output should fail
    # here with a clear message, not inside ci_lint's own parsing.
    try:
        json.loads(plan_json)
        json.loads(needs_json)
    except json.JSONDecodeError as exc:
        print(
            f"ci/ci_ok.py: CI_OK_PLAN_JSON/CI_OK_NEEDS_JSON is not valid JSON: {exc}",
            file=sys.stderr,
        )
        return 2

    plan_path = Path(os.environ.get("RUNNER_TEMP", "/tmp")) / "ci-ok-plan.json"
    needs_path = Path(os.environ.get("RUNNER_TEMP", "/tmp")) / "ci-ok-needs.json"
    plan_path.write_text(plan_json, encoding="utf-8")
    needs_path.write_text(needs_json, encoding="utf-8")

    reuse_path = _write_if_present("CI_OK_REUSE_JSON", "ci-ok-reuse.json")
    event_path = _write_if_present("CI_OK_EVENT_JSON", "ci-ok-event.json")

    cmd = [
        "python3",
        "-m",
        "ci_lint",
        "gate",
        "--repo",
        str(ROOT),
        "--plan",
        str(plan_path),
        "--needs",
        str(needs_path),
        "--attested-workflow",
        "ci.yml",
    ]
    if reuse_path is not None:
        cmd += ["--reuse", str(reuse_path)]
    if event_path is not None:
        cmd += ["--event", str(event_path)]
    # zackees/ci.yml#162 (GEN-021): a `reuse-check.json` proving a skipped
    # required job. Distinct from `--reuse` above, which proves a lane of
    # this same PR from an earlier attempt of its own head. The path is a
    # runner-local file the workflow downloads, never an interpolated
    # expression. Absent (every PR run, and any run with no decision) simply
    # means no skip is credited.
    db_reuse = os.environ.get("CI_OK_DEFAULT_BRANCH_REUSE", "").strip()
    if db_reuse:
        if not Path(db_reuse).is_file():
            print(
                f"ci/ci_ok.py: CI_OK_DEFAULT_BRANCH_REUSE points at {db_reuse!r}, "
                "which is not a file -- refusing to run the gate",
                file=sys.stderr,
            )
            return 2
        cmd += ["--default-branch-reuse", db_reuse]
    print(f"+ {' '.join(cmd)}", flush=True)
    proc = subprocess.run(cmd, cwd=ROOT, check=False)
    return proc.returncode


if __name__ == "__main__":
    raise SystemExit(main())
