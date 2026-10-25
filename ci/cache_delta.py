"""PR compile-delta cache orchestration (zackees/ci.yml#6 §6, round 4C).

Probe evidence (PR #31, push 1, run 36513531709, job 109230756564):
saved `delta-v1-pr31-compile-linux-x64-ba37ca145-gcf0a8151` (76 files,
1,215,988 B) after a clean miss ("Cache not found for input keys:
delta-v1-pr31-compile-linux-x64-ba37ca145-gcf0a8151,
delta-v1-pr31-compile-linux-x64-ba37ca145-"). This second commit is push
2 of the round's probe -- a genuinely NEW commit, not a re-run -- to
settle whether it restores push 1's saved entry by the `-b<base8>-`
prefix (issue #6 §6: GitHub's docs say a PR-scoped entry "can only be
restored by re-runs of the pull request").

"A small delta, never a base": on a PR push, `fast`'s compile cache is
setup-soldr's own `main`-scope base (restored, never re-saved by a PR) plus
a small per-PR delta this module manages -- the zccache compile units this
PR's own commits produced that are absent from that base. The delta is
capped (`[cache.pr].max-per-pr`), gated by `ci_lint cache save-ok`
(CACHE-008, issue #6 §6 "when we do NOT save"), and lives under ONE
GitHub Actions cache key per PR/family/platform generation -- because cache
entries are immutable, each save gets a `-g<sha8>` commit-sha suffix, and
the PREVIOUS generation is deleted (`ci_lint cache heal`) right after a new
one saves successfully, so exactly one survives at a time.

`ci_lint`'s own CLI (`cache key`/`delta manifest|pack|apply`/`save-ok`/
`heal`) does all the real work; every subcommand here is a thin orchestrator
around subprocess calls to it plus GITHUB_OUTPUT plumbing, keeping every
`.github/workflows/ci.yml` `run:` line a single `python3 ci/cache_delta.py
<subcommand>` call (CLAUDE.md rule 6 / GEN-005).

The `actions/cache/restore`+`/save` calls themselves are NOT here -- those
must be literal `uses: ./.github/actions/cache` workflow steps (composite
actions cannot be invoked from a plain script) -- this module only computes
their inputs and consumes their outputs (passed in as CLI args / read from
GITHUB_OUTPUT-adjacent env by the caller).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CI_LINT_MODULE = ["python3", "-m", "ci_lint"]
PACK_COUNT_RE = re.compile(r"packed (\d+) file\(s\)")


def _run(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    print(f"+ {' '.join(cmd)}", flush=True)
    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, check=False)
    if proc.stdout:
        print(proc.stdout, end="")
    if proc.stderr:
        print(proc.stderr, end="", file=sys.stderr)
    if check and proc.returncode != 0:
        raise SystemExit(proc.returncode)
    return proc


def _github_output(pairs: dict[str, str]) -> None:
    gh_out = os.environ.get("GITHUB_OUTPUT")
    if not gh_out:
        for k, v in pairs.items():
            print(f"(no GITHUB_OUTPUT set) {k}={v}")
        return
    with open(gh_out, "a", encoding="utf-8") as fh:
        for k, v in pairs.items():
            fh.write(f"{k}={v}\n")


def _short_sha() -> str:
    sha = os.environ.get("GITHUB_SHA", "")
    if len(sha) >= 8:
        return sha[:8]
    proc = subprocess.run(
        ["git", "rev-parse", "--short=8", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    return proc.stdout.strip() or hashlib.sha256(os.urandom(8)).hexdigest()[:8]


def cmd_prep(args: argparse.Namespace) -> int:
    """After setup-soldr's own base restore: snapshot the base manifest
    (the pristine main-scope state, BEFORE any delta is applied) and build
    this generation's exact key + the prefix a previous generation would be
    restored under."""

    Path(args.archive_path).parent.mkdir(parents=True, exist_ok=True)
    Path(args.manifest_path).parent.mkdir(parents=True, exist_ok=True)

    _run(
        CI_LINT_MODULE
        + [
            "cache",
            "delta",
            "manifest",
            "--dir",
            args.build_cache_path,
            "--out",
            args.manifest_path,
        ]
    )

    key_proc = _run(
        CI_LINT_MODULE
        + [
            "cache",
            "key",
            args.family,
            "--repo",
            args.repo,
            "--platform",
            args.platform,
            "--pr",
            str(args.pr),
            "--base-key",
            args.base_key,
        ]
    )
    base_delta_key = key_proc.stdout.strip().splitlines()[-1]
    sha8 = _short_sha()
    exact_key = f"{base_delta_key}-g{sha8}"
    restore_prefix = f"{base_delta_key}-"

    print(
        f"cache_delta prep: base-key={args.base_key!r} delta-key={exact_key!r} restore-prefix={restore_prefix!r}"
    )
    _github_output(
        {
            "key": exact_key,
            "restore-key-prefix": restore_prefix,
            "base-manifest-path": args.manifest_path,
            "delta-archive-path": args.archive_path,
            "base-key": args.base_key,
        }
    )
    return 0


def cmd_apply(args: argparse.Namespace) -> int:
    """Overlay a restored delta archive onto the just-restored base
    (before the build runs). A stale base (exit 3) is logged and treated
    as a miss -- self-heal, never a job failure (issue #6 §6)."""

    if not Path(args.delta_archive).is_file():
        print(
            f"cache_delta apply: no delta archive at {args.delta_archive} (restore missed) -- nothing to apply"
        )
        return 0
    proc = _run(
        CI_LINT_MODULE
        + [
            "cache",
            "delta",
            "apply",
            "--dir",
            args.build_cache_path,
            "--delta",
            args.delta_archive,
            "--base-manifest",
            args.base_manifest,
        ],
        check=False,
    )
    if proc.returncode == 3:
        print(
            "cache_delta apply: stale base (main moved since this delta was saved) -- treated as miss, continuing cold"
        )
        return 0
    if proc.returncode != 0:
        raise SystemExit(proc.returncode)
    return 0


def cmd_gate(args: argparse.Namespace) -> int:
    """After the build: pack the delta (everything in build-cache-path not
    in the pristine base manifest -- the PR's cumulative delta payload,
    replacing the previous generation wholesale, not a diff-of-diffs), then
    evaluate `cache save-ok` (CACHE-008) to decide whether to save it."""

    pack_proc = _run(
        CI_LINT_MODULE
        + [
            "cache",
            "delta",
            "pack",
            "--dir",
            args.build_cache_path,
            "--base-manifest",
            args.base_manifest,
            "--out",
            args.archive_path,
            "--family",
            args.family,
            "--platform",
            args.platform,
            "--pr",
            str(args.pr),
        ]
    )
    m = PACK_COUNT_RE.search(pack_proc.stdout)
    new_units = int(m.group(1)) if m else 0
    payload_bytes = (
        Path(args.archive_path).stat().st_size
        if Path(args.archive_path).is_file()
        else 0
    )
    print(f"cache_delta gate: new_units={new_units} payload_bytes={payload_bytes}")

    save_ok_cmd = CI_LINT_MODULE + [
        "cache",
        "save-ok",
        args.family,
        "--repo",
        args.repo,
        "--flow",
        args.flow,
        "--event",
        args.event,
        "--pr",
        str(args.pr),
        "--new-units",
        str(new_units),
        "--payload-bytes",
        str(payload_bytes),
        "--base-key",
        args.base_key,
        "--current-base-key",
        args.base_key,
        "--json",
    ]
    if args.lockfile_changed.strip().lower() == "true":
        save_ok_cmd.append("--lockfile-changed")
    if args.title:
        save_ok_cmd.extend(["--tags", args.title])
    save_ok_proc = _run(save_ok_cmd, check=False)
    if save_ok_proc.returncode == 2:
        raise SystemExit(2)
    result = json.loads(save_ok_proc.stdout)
    print(f"cache_delta gate: save-ok -> {result}")
    _github_output(
        {
            "should-save": "true" if result["save"] else "false",
            "reason": str(result["reason"]),
        }
    )
    return 0


def cmd_cleanup(args: argparse.Namespace) -> int:
    """After a successful save: delete the previous generation's exact key
    (if any, and if distinct from the new one) so exactly one delta
    generation survives per PR/family/platform -- cache entries are
    immutable, so this is the retention mechanism, not a `save` overwrite."""

    if not args.previous_key or args.previous_key == args.new_key:
        print(
            f"cache_delta cleanup: nothing to heal (previous={args.previous_key!r} new={args.new_key!r})"
        )
        return 0
    _run(CI_LINT_MODULE + ["cache", "heal", "--key", args.previous_key])
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="cache_delta")
    sub = p.add_subparsers(dest="command", required=True)

    p_prep = sub.add_parser("prep")
    p_prep.add_argument("--repo", default=".")
    p_prep.add_argument("--family", required=True)
    p_prep.add_argument("--platform", required=True)
    p_prep.add_argument("--pr", required=True)
    p_prep.add_argument("--base-key", required=True)
    p_prep.add_argument("--build-cache-path", required=True)
    p_prep.add_argument("--archive-path", required=True)
    p_prep.add_argument("--manifest-path", required=True)
    p_prep.set_defaults(func=cmd_prep)

    p_apply = sub.add_parser("apply")
    p_apply.add_argument("--build-cache-path", required=True)
    p_apply.add_argument("--delta-archive", required=True)
    p_apply.add_argument("--base-manifest", required=True)
    p_apply.set_defaults(func=cmd_apply)

    p_gate = sub.add_parser("gate")
    p_gate.add_argument("--repo", default=".")
    p_gate.add_argument("--family", required=True)
    p_gate.add_argument("--platform", required=True)
    p_gate.add_argument("--pr", required=True)
    p_gate.add_argument("--flow", required=True)
    p_gate.add_argument("--event", required=True)
    p_gate.add_argument("--base-key", required=True)
    p_gate.add_argument("--build-cache-path", required=True)
    p_gate.add_argument("--base-manifest", required=True)
    p_gate.add_argument("--archive-path", required=True)
    p_gate.add_argument("--title", default="")
    p_gate.add_argument(
        "--lockfile-changed",
        default="false",
        help='"true" or "false" (a string, not a flag -- keeps the caller a single run: line)',
    )
    p_gate.set_defaults(func=cmd_gate)

    p_cleanup = sub.add_parser("cleanup")
    p_cleanup.add_argument("--previous-key", default="")
    p_cleanup.add_argument("--new-key", required=True)
    p_cleanup.set_defaults(func=cmd_cleanup)

    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
