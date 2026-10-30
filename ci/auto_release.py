#!/usr/bin/env python3
"""Automatic release: a version bump on main ships itself.

    python3 ci/auto_release.py detect             # auto-release.yml `prepare`
    python3 ci/auto_release.py dispatch-candidate # prepare: start the release flow
    python3 ci/auto_release.py handoff            # ci.yml `release-handoff`
    python3 ci/auto_release.py github-release     # auto-release.yml `github-release`
    python3 ci/auto_release.py prune-dist         # auto-release.yml `publish`

The chain (no `workflow_run`, SEC-003; every hop is an explicit dispatch):

1. A **push** to main that changes `[workspace.package].version` runs
   `auto-release.yml`. `detect` says ``candidate``; `dispatch-candidate`
   dispatches `ci.yml` with ``sha=<bump commit>``: the full release flow
   (every platform wheel, smokes, ``ci-lint release verify``), which
   uploads ``release-dist-verified``.
2. That run's last job, `release-handoff`, runs only once release-verify
   and the OIDC proof succeeded, and dispatches `auto-release.yml` with
   ``candidate_sha`` and ``full_ci_run_id`` (its own run id).
3. `detect` checks that run through the API (ci.yml, workflow_dispatch,
   titled ``release <sha>``, release-verify green) and says ``publish``:
   `github-release` tags and attaches the dist, then `publish` uploads the
   same bytes to PyPI over OIDC. The dist is downloaded by run id and
   never rebuilt (REL-005).

A maintainer can run step 3 by hand: dispatch `auto-release.yml` with a
candidate sha and the id of a green release-flow run for it.

Fails closed: any git/API error raises rather than guessing. Stdlib only,
self-contained, like every other `ci/*.py` module here.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

import tomllib

SEMVER = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?")
SHA = re.compile(r"[0-9a-f]{40}")
RESERVATION = "0.0.0"
CI_WORKFLOW = ".github/workflows/ci.yml"
VERIFY_JOB = "release-verify"


@dataclass(frozen=True)
class Decision:
    action: str
    sha: str = ""
    version: str = ""
    tag: str = ""
    run_id: str = ""
    reason: str = ""


def workspace_version(cargo_toml: str) -> str:
    version = tomllib.loads(cargo_toml)["workspace"]["package"]["version"]
    if not SEMVER.fullmatch(version):
        raise ValueError(f"Cargo.toml version {version!r} is not semantic")
    return version


def project_version(pyproject_toml: str) -> str:
    return tomllib.loads(pyproject_toml)["project"]["version"]


def run_problem(run: dict, jobs: list[dict], sha: str) -> str | None:
    """Why `run` is not a green release-flow run for `sha`, or None."""
    if run.get("path") != CI_WORKFLOW:
        return f"run is {run.get('path')!r}, not {CI_WORKFLOW}"
    if run.get("event") != "workflow_dispatch":
        return f"run event is {run.get('event')!r}, not workflow_dispatch"
    if run.get("display_title") != f"release {sha}":
        return f"run is titled {run.get('display_title')!r}, not 'release {sha}'"
    verify = [j for j in jobs if j.get("name") == VERIFY_JOB]
    if not verify or verify[0].get("conclusion") != "success":
        return f"{VERIFY_JOB} did not succeed in that run"
    return None


def _candidate(
    event: str, head_sha: str, candidate_sha: str, run_id: str, show
) -> tuple[str, str] | Decision:
    """`(sha, version)` this event proposes to release, or a `none` Decision."""
    if event == "push":
        if not SHA.fullmatch(head_sha):
            raise ValueError(f"bad head sha {head_sha!r}")
        version = workspace_version(show(head_sha, "Cargo.toml"))
        parent = show(f"{head_sha}^", "Cargo.toml")
        # A root commit is the repository's creation, not a bump: the first
        # release is cut by bumping, or by dispatching auto-release.yml.
        if parent is None:
            return Decision(
                "none",
                reason=f"{head_sha[:12]} has no parent Cargo.toml (initial commit)",
            )
        if workspace_version(parent) == version:
            return Decision(
                "none", reason=f"version {version} unchanged by {head_sha[:12]}"
            )
        return head_sha, version
    if event == "workflow_dispatch":
        if not SHA.fullmatch(candidate_sha):
            raise ValueError(f"candidate_sha {candidate_sha!r} is not a 40-hex commit")
        if not run_id.isdigit():
            raise ValueError(f"full_ci_run_id {run_id!r} is not a run id")
        return candidate_sha, workspace_version(show(candidate_sha, "Cargo.toml"))
    return Decision("none", reason=f"event {event!r} never releases")


def decide(  # pylint: disable=too-many-arguments  # keyword-only inputs of one decision
    *,
    event: str,
    ref: str,
    head_sha: str,
    candidate_sha: str,
    run_id: str,
    show,
    tag_exists,
    fetch_run,
) -> Decision:
    """Pure decision logic. I/O is injected: `show(sha, path)` -> text or
    None, `tag_exists(tag)` -> bool, `fetch_run(run_id)` -> (run, jobs)."""
    if ref != "refs/heads/main":
        return Decision("none", reason=f"ref {ref!r} is not main")
    candidate = _candidate(event, head_sha, candidate_sha, run_id, show)
    if isinstance(candidate, Decision):
        return candidate
    sha, version = candidate
    if version == RESERVATION:
        return Decision(
            "none", reason="0.0.0 is the name reservation and is never released"
        )
    pyproject = show(sha, "pyproject.toml")
    if pyproject is not None and project_version(pyproject) != version:
        raise ValueError(
            f"pyproject.toml version {project_version(pyproject)!r} != Cargo.toml {version!r}"
        )
    tag = f"v{version}"
    if tag_exists(tag):
        return Decision(
            "none",
            sha=sha,
            version=version,
            tag=tag,
            reason=f"{tag} is already released",
        )
    if event == "push":
        return Decision(
            "candidate",
            sha=sha,
            version=version,
            tag=tag,
            reason=f"bump to {version} at {sha[:12]}",
        )
    problem = run_problem(*fetch_run(run_id), sha)
    if problem:
        raise ValueError(f"refusing to publish {tag} from run {run_id}: {problem}")
    return Decision(
        "publish",
        sha=sha,
        version=version,
        tag=tag,
        run_id=run_id,
        reason=f"publish {tag} from run {run_id}",
    )


# --- I/O -------------------------------------------------------------------


def _git_show(sha: str, path: str) -> str | None:
    proc = subprocess.run(
        ["git", "show", f"{sha}:{path}"], capture_output=True, text=True, check=False
    )
    if proc.returncode == 0:
        return proc.stdout
    if re.search(
        r"does not exist|unknown revision|bad revision|invalid object name", proc.stderr
    ):
        return None
    raise RuntimeError(f"git show {sha}:{path} failed: {proc.stderr.strip()}")


def _gh(*args: str) -> str:
    return subprocess.run(
        ["gh", *args], capture_output=True, text=True, check=True
    ).stdout


def _tag_exists(tag: str) -> bool:
    repo = os.environ["GITHUB_REPOSITORY"]
    proc = subprocess.run(
        ["gh", "api", f"repos/{repo}/git/ref/tags/{tag}"],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode == 0:
        return True
    if "HTTP 404" in proc.stderr or "Not Found" in proc.stderr:
        return False
    raise RuntimeError(f"tag lookup for {tag} failed: {proc.stderr.strip()}")


def _fetch_run(run_id: str) -> tuple[dict, list[dict]]:
    repo = os.environ["GITHUB_REPOSITORY"]
    run = json.loads(_gh("api", f"repos/{repo}/actions/runs/{run_id}"))
    # gh rejects `--slurp` together with `--jq`: slurp the pages, flatten here.
    pages = json.loads(
        _gh(
            "api",
            "--paginate",
            f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100",
            "--slurp",
        )
    )
    jobs = [job for page in pages for job in page["jobs"]]
    return run, jobs


def _output(**values: str) -> None:
    lines = "".join(f"{k}={v}\n" for k, v in values.items())
    if out := os.environ.get("GITHUB_OUTPUT"):
        with open(out, "a", encoding="utf-8") as fh:
            fh.write(lines)
    else:
        print(lines, end="")


def cmd_detect() -> None:
    env = os.environ
    d = decide(
        event=env["GITHUB_EVENT_NAME"],
        ref=env["GITHUB_REF"],
        head_sha=env["GITHUB_SHA"],
        candidate_sha=env.get("CANDIDATE_SHA", ""),
        run_id=env.get("FULL_CI_RUN_ID", ""),
        show=_git_show,
        tag_exists=_tag_exists,
        fetch_run=_fetch_run,
    )
    print(f"auto_release: {d.action}: {d.reason}")
    _output(action=d.action, sha=d.sha, version=d.version, tag=d.tag, run_id=d.run_id)


def cmd_dispatch_candidate() -> None:
    sha = os.environ["CANDIDATE_SHA"]
    _gh("workflow", "run", "ci.yml", "--ref", "main", "-f", f"sha={sha}")
    print(f"auto_release: dispatched the release flow (ci.yml) for {sha}")


def cmd_handoff() -> None:
    sha, run_id = os.environ["CANDIDATE_SHA"], os.environ["GITHUB_RUN_ID"]
    _gh(
        "workflow",
        "run",
        "auto-release.yml",
        "--ref",
        "main",
        "-f",
        f"candidate_sha={sha}",
        "-f",
        f"full_ci_run_id={run_id}",
    )
    print(f"auto_release: handed {sha} (run {run_id}) to auto-release.yml")


def _dists(dist: Path) -> list[Path]:
    return sorted(
        p for p in dist.iterdir() if p.suffix == ".whl" or p.name.endswith(".tar.gz")
    )


def cmd_github_release() -> None:
    env = os.environ
    tag, sha = env["TAG"], env["CANDIDATE_SHA"]
    files = _dists(Path("dist"))
    if not files:
        raise RuntimeError("dist/ holds no wheel or sdist")
    notes = f"Verified by run {env['FULL_CI_RUN_ID']}. Install: `pip install ghd=={tag.removeprefix('v')}`."
    _gh(
        "release",
        "create",
        tag,
        *map(str, files),
        "--target",
        sha,
        "--title",
        f"ghd {tag}",
        "--generate-notes",
        "--notes",
        notes,
    )
    print(f"auto_release: released {tag} at {sha} with {len(files)} file(s)")


def cmd_prune_dist() -> None:
    dist = Path("dist")
    keep = set(_dists(dist))
    for path in dist.iterdir():
        if path not in keep:
            path.unlink()
    print("auto_release: uploading " + ", ".join(p.name for p in sorted(keep)))


COMMANDS = {
    "detect": cmd_detect,
    "dispatch-candidate": cmd_dispatch_candidate,
    "handoff": cmd_handoff,
    "github-release": cmd_github_release,
    "prune-dist": cmd_prune_dist,
}


def main(argv: list[str]) -> int:
    if len(argv) != 1 or argv[0] not in COMMANDS:
        print(__doc__, file=sys.stderr)
        return 2
    COMMANDS[argv[0]]()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
