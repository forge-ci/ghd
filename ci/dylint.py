"""Dylint lane orchestration: one Linux job checks every declared target.

ci.yml#9 (setup-soldr v0.9.83+): the Setup-soldr step now takes
`dylint-targets` (this script's cross-target list, wired through
`.github/actions/soldr/action.yml`) and itself runs
`soldr dylint prepare --target T` for every declared target, keyed into
the Dylint foundation/output cache identity. This script therefore no
longer prepares cross targets itself -- only the host materialization
that `dylint: "true"` mode already performs unconditionally. Deleting the
per-target prepare loop here (and the wasted rust-std download it used to
race the check pass against) is the whole point of ci.yml#9: no
per-repository cache plumbing.

zackees/ci.yml#6 §2/§3/§5, and its third comment ("Use case to test:
cross-target Dylint must be cached and fast", D1-D8). This script is the
single `run:` line the `dylint` job in `.github/workflows/ci.yml` calls; it
does the per-target looping so the workflow YAML stays one line per step
(CLAUDE.md rule 6 / `GEN-005`).

Target source: `ci.toml`'s `[platforms]` table, read directly with stdlib
`tomllib` -- NOT `plan.json`'s `dylint_targets` field. Round 1's planner
narrows `dylint_targets` to the flow's *build* platform selection (just
`linux-x64` on an untagged PR), which is right for the build/test lane but
wrong for Dylint: ci.yml#6 says Dylint always covers every declared
platform, tagged or not, because it is a cheap pre-expansion/type check,
not a full cross build. Falling back to `[platforms]` directly is the
explicit brief instruction for this round ("or ci.toml [platforms] until
2A lands") and is reported as a planner finding, not silently patched
around.

Never invokes bare `cargo`/`rustc`/`rustup` -- every Dylint invocation goes
through `soldr` (RUST-001/RUST-002).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import time
from dataclasses import asdict, dataclass
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[1]

# The one platform id whose target the `ubuntu-24.04` runner natively
# compiles for; every other declared platform is a cross check-only pass.
# Keep in sync with ci.toml's `linux-x64` entry.
HOST_PLATFORM_ID = "linux-x64"


@dataclass(frozen=True)
class DylintTarget:
    """One declared platform's Dylint identity (ci.toml `[platforms]`)."""

    platform_id: str
    triple: str
    is_host: bool


@dataclass(frozen=True)
class DylintPassResult:
    """One `soldr cargo dylint` invocation's outcome, for the step summary
    and the D1-D8 evidence table. Typed per AGENTS.md's benchmark-data rule
    (no raw dicts for measurement records)."""

    shape: str
    targets: tuple[str, ...]
    command: tuple[str, ...]
    seconds: float
    returncode: int

    def to_json_dict(self) -> dict[str, object]:
        return {
            "shape": self.shape,
            "targets": list(self.targets),
            "command": list(self.command),
            "seconds": round(self.seconds, 3),
            "returncode": self.returncode,
        }


def _load_platforms(repo: Path) -> list[DylintTarget]:
    ci_toml_path = repo / "ci.toml"
    with ci_toml_path.open("rb") as fh:
        data = tomllib.load(fh)
    platforms = data.get("platforms", {})
    if not isinstance(platforms, dict) or not platforms:
        raise SystemExit(
            f"ci/dylint.py: ci.toml has no [platforms] table at {ci_toml_path}"
        )
    out: list[DylintTarget] = []
    for platform_id, entry in sorted(platforms.items()):
        triple = entry["target"]
        out.append(
            DylintTarget(
                platform_id=platform_id,
                triple=triple,
                is_host=(platform_id == HOST_PLATFORM_ID),
            )
        )
    if not any(t.is_host for t in out):
        raise SystemExit(
            f"ci/dylint.py: ci.toml [platforms] has no '{HOST_PLATFORM_ID}' entry"
        )
    return out


def _dylint_env() -> dict[str, str]:
    """Every `soldr dylint ...` invocation (prepare or check) must run
    with `RUSTUP_TOOLCHAIN` unset, mirroring zackees/clud's working
    `_dylint.yml` (`env -u RUSTUP_TOOLCHAIN soldr dylint ...`). setup-soldr's
    `dylint: true` mode exports `RUSTUP_TOOLCHAIN` for the workspace's own
    stable channel; left set, it overrides the nightly `soldr dylint`
    itself resolves for the driver, and the nested build/check fails to
    find a repo-pinned toolchain in its own isolated temp directory. A
    one-line `run:` step can't chain `env -u ... &&`, so the unset happens
    here instead."""
    env = dict(os.environ)
    env.pop("RUSTUP_TOOLCHAIN", None)
    return env


def _run(cmd: list[str]) -> tuple[int, float]:
    print(f"+ {' '.join(cmd)}", flush=True)
    start = time.monotonic()
    proc = subprocess.run(cmd, cwd=ROOT, env=_dylint_env(), check=False)
    return proc.returncode, time.monotonic() - start


def _check_cmd(target: DylintTarget | None) -> list[str]:
    """`soldr dylint` (soldr's own subcommand, not `soldr cargo dylint`,
    which defers to a stray `cargo-dylint` on PATH instead of soldr's
    managed/cached one) `--all -- --workspace --all-targets [--target T]`
    -- soldr's own flag first, cargo-dylint's arguments after `--`. Matches
    zackees/clud's proven working `_dylint.yml` invocation exactly."""
    cmd = ["soldr", "dylint", "--all", "--", "--workspace", "--all-targets"]
    if target is not None:
        cmd += ["--target", target.triple]
    return cmd


def _run_sequential(
    host: DylintTarget, cross: list[DylintTarget]
) -> list[DylintPassResult]:
    results: list[DylintPassResult] = []
    host_cmd = _check_cmd(None)
    rc, seconds = _run(host_cmd)
    results.append(
        DylintPassResult("sequential", (host.triple,), tuple(host_cmd), seconds, rc)
    )
    if rc != 0:
        return results
    for target in cross:
        cmd = _check_cmd(target)
        rc, seconds = _run(cmd)
        results.append(
            DylintPassResult("sequential", (target.triple,), tuple(cmd), seconds, rc)
        )
        if rc != 0:
            return results
    return results


def _run_multi_target(
    host: DylintTarget, cross: list[DylintTarget]
) -> list[DylintPassResult]:
    """D6 candidate: host artifacts (proc-macros, build scripts) compile
    once for every target in a single invocation, rather than once per
    sequential pass (see the clud macOS-vs-Windows half-cost hint in
    ci.yml#6 comment 3). Not proven upstream (clud only ever runs
    sequential, one `--target` per invocation); measured here and picked
    only if it is both correct and faster."""
    cmd = ["soldr", "dylint", "--all", "--", "--workspace", "--all-targets"]
    for target in cross:
        cmd += ["--target", target.triple]
    rc, seconds = _run(cmd)
    all_triples = tuple([host.triple] + [t.triple for t in cross])
    return [DylintPassResult("multi-target", all_triples, tuple(cmd), seconds, rc)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default=str(ROOT))
    # D6 (ci.yml#6 round 3B, measured on run 36501033796 vs 36500282280):
    # multi-target is 37% faster on the check passes themselves (55.5s vs
    # 88.8s) and 23% faster end to end (114s vs 148s job total) -- see
    # ci.toml [lint.dylint]'s comment for the full numbers.
    parser.add_argument(
        "--shape", choices=["sequential", "multi-target"], default="multi-target"
    )
    parser.add_argument(
        "--results-out", default=None, help="write DylintPassResult[] JSON here"
    )
    parser.add_argument(
        "--print-cross-targets",
        action="store_true",
        help=(
            "Print 'targets=<comma-separated cross triples>' to stdout "
            "(GITHUB_OUTPUT format) and exit; used by the workflow step "
            "that resolves the Setup-soldr `dylint-targets` input before "
            "the check pass runs, so setup-soldr can prepare rust-std for "
            "every declared cross target ahead of time (ci.yml#9)."
        ),
    )
    args = parser.parse_args(argv)
    repo = Path(args.repo).resolve()

    platforms = _load_platforms(repo)
    host = next(t for t in platforms if t.is_host)
    cross = [t for t in platforms if not t.is_host]

    if args.print_cross_targets:
        print(f"targets={','.join(t.triple for t in cross)}")
        return 0

    print(
        f"dylint lane: host={host.triple} cross={[t.triple for t in cross]} shape={args.shape}"
    )

    if args.shape == "multi-target":
        results = _run_multi_target(host, cross)
    else:
        results = _run_sequential(host, cross)

    total_seconds = sum(r.seconds for r in results)
    print(f"dylint lane: {len(results)} pass(es), {total_seconds:.1f}s total")
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write(f"\n### Dylint lane ({args.shape})\n\n")
            fh.write("| targets | seconds | rc |\n|---|---|---|\n")
            for r in results:
                fh.write(
                    f"| {', '.join(r.targets)} | {r.seconds:.1f} | {r.returncode} |\n"
                )
            fh.write(f"\n**total**: {total_seconds:.1f}s\n")

    if args.results_out:
        Path(args.results_out).write_text(
            json.dumps([asdict(r) for r in results], indent=2), encoding="utf-8"
        )

    return next((r.returncode for r in results if r.returncode != 0), 0)


if __name__ == "__main__":
    raise SystemExit(main())
