"""Clone/fetch the pinned `ci_lint` checkout used by every local command.

`ci.toml`'s `linter` field (`zackees/ci.yml@<sha>`) is the single source
of truth for which `ci_lint` commit precheck runs -- the same pin
`.github/workflows/ci-pre.yml` and `ci.yml` check out in CI
(`CT-004`). Locally we keep one gitignored checkout at `.ci-lint/` (the
same directory CI would produce if it ran here) and fast-path to a no-op
when it is already at the pinned commit, so a warm `precheck` run pays no
network cost at all.
"""

from __future__ import annotations

import os
import re
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

import tomllib

CI_LINT_REMOTE = "https://github.com/zackees/ci.yml.git"
_LINTER_RE = re.compile(r"^([\w.-]+/[\w.-]+)@([0-9a-fA-F]{40})$")


class CiLintCheckoutError(RuntimeError):
    pass


@dataclass(frozen=True)
class CheckoutResult:
    path: Path
    sha: str
    was_cold: bool  # True: a clone/fetch actually ran; False: already warm at `sha`.
    seconds: float


def pinned_sha(repo_root: Path) -> str:
    """Read `ci.toml`'s `linter = "owner/repo@<40-hex-sha>"` field."""

    ci_toml_path = repo_root / "ci.toml"
    try:
        raw = ci_toml_path.read_bytes()
    except OSError as exc:
        raise CiLintCheckoutError(f"cannot read {ci_toml_path}: {exc}") from exc
    try:
        doc = tomllib.loads(raw.decode("utf-8"))
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as exc:
        raise CiLintCheckoutError(f"cannot parse {ci_toml_path}: {exc}") from exc
    linter = doc.get("linter")
    if not isinstance(linter, str):
        raise CiLintCheckoutError(f"{ci_toml_path} has no string `linter` field")
    m = _LINTER_RE.match(linter)
    if not m:
        raise CiLintCheckoutError(
            f"{ci_toml_path} `linter = {linter!r}` does not match 'owner/repo@<40-hex-sha>'"
        )
    return m.group(2)


@dataclass(frozen=True)
class GitResult:
    returncode: int
    stdout: str
    stderr: str


def _run(cmd: list[str], *, cwd: Path) -> GitResult:
    env = os.environ.copy()
    env["GIT_TERMINAL_PROMPT"] = "0"
    with (
        tempfile.TemporaryFile(mode="w+", encoding="utf-8", errors="replace") as stdout,
        tempfile.TemporaryFile(mode="w+", encoding="utf-8", errors="replace") as stderr,
    ):
        result = subprocess.run(
            cmd, cwd=cwd, stdout=stdout, stderr=stderr, check=False, env=env
        )
        stdout.seek(0)
        stderr.seek(0)
        return GitResult(result.returncode, stdout.read(), stderr.read())


def ensure_ci_lint(
    repo_root: Path, *, target_dir: Path | None = None
) -> CheckoutResult:
    """Ensure `target_dir` (default `repo_root/.ci-lint`) is a checkout of
    the `ci.toml`-pinned SHA (`ci.toml` is always read from `repo_root`,
    even when `target_dir` points elsewhere).

    Warm path (already at the pinned SHA): one `git rev-parse HEAD`, no
    network. Cold path: `git init` + `git fetch --depth 1 <sha>` +
    `git checkout FETCH_HEAD` -- GitHub.com serves arbitrary reachable
    commit SHAs over the smart HTTP protocol, so this works without a
    branch name.

    `target_dir` exists for `act_inner.py` (round-4B act parity, part 2):
    `.ci-lint/` is gitignored, and act's default (non `--bind`) copy of
    the working directory into each job container is git-aware -- it
    silently DROPS every gitignored path (confirmed empirically: `--bind`
    itself is not viable here either, since `act` runs INSIDE the bosn
    act-stack container, and `--bind`'s host-path bind-mount is resolved
    by the TRUE host's docker daemon, which has no path called `/work` --
    every file, not just `.ci-lint/`, came back missing under `--bind`).
    The fix is a machine-scoped bosn volume (`bosn.toml`'s
    `[stack.act.volumes.ci-lint]`) populated by THIS function at a path
    outside the git working tree, then bind-mounted into each job
    container by Docker volume NAME (global to the daemon, so it needs no
    path translation) via `act`'s `--container-options`.
    """

    start = time.monotonic()
    sha = pinned_sha(repo_root)
    ci_lint_dir = target_dir if target_dir is not None else repo_root / ".ci-lint"

    if (ci_lint_dir / ".git").is_dir():
        current = _run(["git", "rev-parse", "HEAD"], cwd=ci_lint_dir)
        if current.returncode == 0 and current.stdout.strip() == sha:
            return CheckoutResult(
                ci_lint_dir, sha, was_cold=False, seconds=time.monotonic() - start
            )

    ci_lint_dir.mkdir(parents=True, exist_ok=True)
    if not (ci_lint_dir / ".git").is_dir():
        init = _run(["git", "init", "-q", "-b", "ci-lint-pin"], cwd=ci_lint_dir)
        if init.returncode != 0:
            raise CiLintCheckoutError(
                f"git init failed in {ci_lint_dir}: {init.stderr}"
            )
        remote = _run(
            ["git", "remote", "add", "origin", CI_LINT_REMOTE], cwd=ci_lint_dir
        )
        if remote.returncode != 0:
            raise CiLintCheckoutError(
                f"git remote add failed in {ci_lint_dir}: {remote.stderr}"
            )

    fetch = _run(["git", "fetch", "--depth", "1", "origin", sha], cwd=ci_lint_dir)
    if fetch.returncode != 0:
        raise CiLintCheckoutError(
            f"git fetch of zackees/ci.yml@{sha} failed: {fetch.stderr.strip()}"
        )
    checkout = _run(
        ["git", "checkout", "-q", "--detach", "FETCH_HEAD"], cwd=ci_lint_dir
    )
    if checkout.returncode != 0:
        raise CiLintCheckoutError(
            f"git checkout of {sha} failed: {checkout.stderr.strip()}"
        )

    return CheckoutResult(
        ci_lint_dir, sha, was_cold=True, seconds=time.monotonic() - start
    )
