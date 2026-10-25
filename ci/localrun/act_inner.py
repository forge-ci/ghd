"""`python3 ci/local.py act-inner`: runs INSIDE the bosn `act` stack container.

Never invoked directly -- `bosn.toml`'s `[task.act-run]` is the only
caller (`ci/local.py act`, on the host, triggers it via `bosn run --task
act-run`). Reads `.act-local/request.json` (written by the host step,
visible here because the repo is bind-mounted at `/work` == the host's
checkout root), builds the clang-patched runner image if it is not
already present (zackees/ci.yml#6 section 11's D7: `catthehacker/ubuntu
:act-24.04` has no `clang`, which setup-soldr's linker shim needs), runs
`act` once per requested lane against the host Docker engine (this
container's bind-mounted `/var/run/docker.sock`), labels and removes the
sibling job containers it creates, then runs the ACT-001 cache audit
against the machine-scoped cache-server volume and writes
`.act-local/result.json` for the host step to read back.

Resolved gap (round-4B act parity; see README "GITHUB_TOKEN and
cross-repo checkouts" for the history): `ci.yml`/`ci-pre.yml`'s own
"Checkout ci-lint (zackees/ci.yml, pinned)" steps are skipped under act
(`if: env.ACT != 'true'`) instead of hitting `actions/checkout`'s
`repository:`-override token requirement (act does not auto-populate
`github.token`). The pinned checkout this tool provides instead --
`ensure_ci_lint` into the machine-scoped `ci-lint` volume below, mounted
by NAME into each job container -- needs no token, since it is a plain
anonymous git clone of a public repo, same as the host-side one
`ci_lint_checkout.py` already does for `python3 ci/local.py precheck`.
This tool still never fetches, embeds, prints, or writes a credential
anywhere (worker contract: no secrets); a job that needs some OTHER
cross-repo action with real auth still falls back to act's documented
`--secret-file` opt-in (`.secrets` in the working directory, i.e.
`/work/.secrets` -- gitignored, never created by this tool, and this
module's own `act` invocation below sets no `--secret-file` override,
so act's stock default applies automatically).
"""

from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path

from ci.localrun.cache_audit import audit as audit_cache
from ci.localrun.cache_audit import render as render_audit
from ci.localrun.ci_lint_checkout import CiLintCheckoutError, ensure_ci_lint
from ci.localrun.ci_toml_lite import CiTomlLiteError, read_cache_section
from ci.localrun.sizes import parse_size

WORK = Path("/work")
ACTION_CACHE = Path("/root/.cache/act")
CACHE_SERVER = Path("/root/.cache/actcache")
# This container's OWN mount point for bosn.toml's `[stack.act.volumes.
# ci-lint]` -- the SAME named Docker volume also gets mounted, by name,
# into each job container at /work/.ci-lint (see `_run_lane`'s
# `--container-options`); Docker volumes are daemon-global by name, so
# the two different mount PATHS (here vs. inside a job container) share
# the one underlying volume regardless of which container is "outer".
CI_LINT_VOLUME_MOUNT = Path("/root/.cache/ci-lint")
CI_LINT_VOLUME_NAME = "bosn-m-template-act-ci-lint"
ARTIFACT_SERVER = Path("/tmp/act-artifacts")
DOCKERFILE_RUNNER = WORK / "ci" / "docker" / "act" / "Dockerfile.runner"
WORKFLOW = ".github/workflows/ci.yml"


def _ensure_runner_image(tag: str) -> float:
    start = time.monotonic()
    inspect = subprocess.run(
        ["docker", "image", "inspect", tag], capture_output=True, check=False
    )
    if inspect.returncode == 0:
        return time.monotonic() - start  # warm: image already present
    build = subprocess.run(
        [
            "docker",
            "build",
            "-f",
            str(DOCKERFILE_RUNNER),
            "-t",
            tag,
            str(DOCKERFILE_RUNNER.parent),
        ],
        check=False,
    )
    if build.returncode != 0:
        raise RuntimeError(
            f"docker build of the runner image ({tag}) failed (exit {build.returncode})"
        )
    return time.monotonic() - start


def _run_lane(
    lane: str,
    *,
    event_path: Path,
    runner_tag: str,
    run_id: str,
    extra_git_mount: str | None,
) -> tuple[float, int]:
    ARTIFACT_SERVER.mkdir(parents=True, exist_ok=True)
    container_options = f"--init --label template.act-run={run_id} -v {CI_LINT_VOLUME_NAME}:/work/.ci-lint"
    if extra_git_mount:
        # zackees/ci.yml#47: `act`'s own Checkout step is a plain `docker
        # cp` of `/work` into each job container, not a real clone -- a
        # linked git worktree's `.git` FILE ("gitdir: <absolute path
        # under the main checkout>") gets copied verbatim, but the path
        # it points at lives outside `/work` and was never mounted, so
        # every `git` command (including `ci_lint`'s tracked-file scan)
        # fails inside the job container. The host-side orchestrator
        # (act_orchestrate.py's `_detect_external_git_common_dir`)
        # detects this and passes the main checkout's absolute root
        # here; bind-mounting it BY IDENTITY (same path, same path)
        # makes the worktree's absolute gitdir reference resolve inside
        # the job container exactly as it does on the host. A no-op for
        # an ordinary (non-worktree) checkout, where this is None.
        container_options += f" -v {extra_git_mount}:{extra_git_mount}"
    cmd = [
        "act",
        "pull_request",
        "-W",
        WORKFLOW,
        "-j",
        lane,
        "-e",
        str(event_path),
        "-P",
        f"ubuntu-24.04={runner_tag}",
        "--pull=false",
        "--rm",
        "--action-cache-path",
        str(ACTION_CACHE),
        "--cache-server-path",
        str(CACHE_SERVER),
        "--artifact-server-path",
        str(ARTIFACT_SERVER),
        "--container-options",
        # `-v NAME:/work/.ci-lint`: mounts the ci-lint volume by NAME
        # (not path -- see CI_LINT_VOLUME_MOUNT's comment) at exactly the
        # path `ci.toml`'s `linter`-pinned checkout normally lands at, so
        # every `PYTHONPATH: .ci-lint`-relative `run:` step works
        # unchanged under act. See above for the optional extra mount.
        container_options,
    ]
    # No --secret-file override: act's own default (".secrets" in this
    # process's cwd, i.e. /work/.secrets) applies. Anonymous by default --
    # this tool never creates, reads, or logs that file. See the module
    # docstring's "Resolved gap" note above.
    start = time.monotonic()
    result = subprocess.run(cmd, cwd=str(WORK), check=False)
    return time.monotonic() - start, result.returncode


def _cleanup_containers(run_id: str) -> None:
    ids = subprocess.run(
        ["docker", "ps", "-aq", "--filter", f"label=template.act-run={run_id}"],
        capture_output=True,
        text=True,
        check=False,
    ).stdout.split()
    if ids:
        print(
            f"[act-inner] removing {len(ids)} job container(s) for run {run_id}",
            file=sys.stderr,
        )
        subprocess.run(["docker", "rm", "-f", *ids], capture_output=True, check=False)


def main(argv: list[str] | None = None) -> int:
    del argv
    request_path = WORK / ".act-local" / "request.json"
    request = json.loads(request_path.read_text(encoding="utf-8"))
    lanes: list[str] = request["lanes"]
    event_path = WORK / request["event_path"]
    run_id: str = request["run_id"]
    runner_tag: str = request["runner_tag"]
    extra_git_mount: str | None = request.get("extra_git_mount")

    result: dict[str, object] = {
        "ok": True,
        "lanes": [],
        "runner_image_build_seconds": None,
    }
    try:
        result["runner_image_build_seconds"] = _ensure_runner_image(runner_tag)
    except RuntimeError as exc:
        print(f"[act-inner] {exc}", file=sys.stderr)
        result["ok"] = False
        (WORK / ".act-local" / "result.json").write_text(
            json.dumps(result, indent=2), encoding="utf-8"
        )
        return 1

    try:
        ci_lint_checkout = ensure_ci_lint(WORK, target_dir=CI_LINT_VOLUME_MOUNT)
        print(
            f"[act-inner] ci-lint volume ({CI_LINT_VOLUME_NAME}) at "
            f"{ci_lint_checkout.sha[:12]}, "
            f"{'cold clone/fetch' if ci_lint_checkout.was_cold else 'warm, no network'} "
            f"({ci_lint_checkout.seconds:.2f}s)",
            file=sys.stderr,
        )
    except CiLintCheckoutError as exc:
        print(f"[act-inner] {exc}", file=sys.stderr)
        result["ok"] = False
        (WORK / ".act-local" / "result.json").write_text(
            json.dumps(result, indent=2), encoding="utf-8"
        )
        return 1

    try:
        for lane in lanes:
            seconds, exit_code = _run_lane(
                lane,
                event_path=event_path,
                runner_tag=runner_tag,
                run_id=run_id,
                extra_git_mount=extra_git_mount,
            )
            succeeded = exit_code == 0
            result["lanes"].append(
                {
                    "id": lane,
                    "seconds": seconds,
                    "exit_code": exit_code,
                    "succeeded": succeeded,
                }
            )
            if not succeeded:
                result["ok"] = False
    finally:
        _cleanup_containers(run_id)

    try:
        cache_cfg = read_cache_section(WORK)
        budget_bytes = parse_size(cache_cfg.budget)
        if budget_bytes is None:
            raise CiTomlLiteError(
                f"ci.toml [cache].budget {cache_cfg.budget!r} does not parse"
            )
        audit_result = audit_cache(
            CACHE_SERVER, budget_bytes=budget_bytes, retired_families=cache_cfg.retired
        )
        rendered = render_audit(audit_result)
        print(rendered)
        result["cache_audit"] = {
            "ok": audit_result.ok,
            "disk_bytes": audit_result.disk_bytes,
            "budget_bytes": audit_result.budget_bytes,
            "findings": list(audit_result.findings),
            "rendered": rendered,
        }
        if not audit_result.ok:
            result["ok"] = False
    except CiTomlLiteError as exc:
        print(f"[act-inner] cache audit could not run: {exc}", file=sys.stderr)
        result["ok"] = False

    (WORK / ".act-local" / "result.json").write_text(
        json.dumps(result, indent=2), encoding="utf-8"
    )
    return 0 if result["ok"] else 1
