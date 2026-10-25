"""Platform-build lane orchestration: cross-compile one platform's test
executables, native CLI, and wheel, all on Linux, then stage them for the
matching `platform-run` leg to download and execute with no Rust toolchain
on the runner (zackees/ci.yml#6 round 3, deliverable 3).

Each `.github/workflows/ci.yml` `platform-build` matrix leg's steps are
one line each calling `python3 ci/platform_build.py <subcommand>`
(CLAUDE.md rule 6 / `GEN-005`); this module holds the logic.

Decisions this round made (see the PR body / worker report for the full
evidence):
  - The wheel is built with `soldr wheel [--release] --target <triple>`
    (zackees/ci.yml#17, ghd#37). Round 3 used `uv
    build --wheel --config-setting target=<triple>` because `soldr wheel`
    ignored `[tool.soldr.pep517] bundle-bins` and would have shipped a
    wheel without the native CLI (PKG-003). zackees/soldr#3468 (soldr
    0.9.27) fixed that: `soldr wheel` now stages bundle-bins through the
    PEP 517 backend's own `_bundle_bins.py`, building each bin with
    `soldr build --target <triple>`, the same catalogue toolchain as the
    extension.
  - Same dev-profile choice the fast lane made (`ci/fast.py`): no
    `--release`/`profile=release` config setting, so `prebuild-deps-flags`
    stays empty and this job's `compile`/`deps` cache families share one
    profile identity across the fast lane and every platform-build lane
    (only the cross target differs). A release-profile wheel matrix is
    `[flow.release]`/`[flow.nightly]` scope (issue #6 §5/§10 round 5), not
    this round's.
  - Test binaries are captured with plain `soldr cargo test --no-run
    --target T --message-format=json` (NOT `soldr ci-test`/nextest
    archives) — matching the fast lane exactly, so `platform-run` only
    ever has to execute a declared, ordinary libtest binary directly, and
    the still-pending soldr#3294 ("replaying nextest archives on foreign
    targets") never becomes this round's problem.

Never invokes bare `cargo`/`rustc`/`maturin`/`pip` (RUST-001/PKG-003).
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import stat
import subprocess
import sys
import zipfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _run(cmd: list[str]) -> int:
    print(f"+ {' '.join(cmd)}", flush=True)
    return subprocess.run(cmd, cwd=ROOT, check=False).returncode


def _run_isolated_soldr(cmd: list[str]) -> int:
    """See `ci/fast.py::_run_isolated_soldr` — identical reasoning (two
    independent soldr broker roots can't share one claim), duplicated
    rather than imported so this module stays a self-contained,
    stdlib-only script like every other `ci/*.py` orchestration module in
    this repo (none of them import each other)."""
    env = {
        k: v for k, v in os.environ.items() if not k.startswith(("SOLDR_", "ZCCACHE_"))
    }
    print(
        f"+ {' '.join(cmd)}  (SOLDR_*/ZCCACHE_* stripped -- see docstring)", flush=True
    )
    return subprocess.run(cmd, cwd=ROOT, env=env, check=False).returncode


def cmd_build_json(args: argparse.Namespace) -> int:
    """Compile (no run) every declared test binary FOR ONE CROSS TARGET,
    capturing pure `--message-format=json` compiler-artifact records —
    same shape `ci/fast.py build-json` produces for the host, just with
    `--target` added so the records (and the resulting executables) are
    this platform's, not the host's.

    `--profile release` (round 5, zackees/ci.yml#6 deliverable 2: the
    `release`/`nightly` flows want release-profile wheels on EVERY
    platform, not just the dev-profile PR/main default) adds `--release`
    to the cargo invocation -- the module docstring's "same dev-profile
    choice" note only ever described the PR/main lanes; it is no longer
    true unconditionally as of this round."""
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        "soldr",
        "cargo",
        "test",
        "--workspace",
        "--locked",
        "--no-run",
        "--target",
        args.target,
    ]
    if args.profile == "release":
        cmd.append("--release")
    cmd.append("--message-format=json")
    print(f"+ {' '.join(cmd)} > {out_path}", flush=True)
    with out_path.open("w", encoding="utf-8") as fh:
        proc = subprocess.run(cmd, cwd=ROOT, stdout=fh, check=False)
    return proc.returncode


def cmd_wheel_build(args: argparse.Namespace) -> int:
    """Cross-build the wheel with `soldr wheel --target <triple>` (plus
    `--release` for `--profile release`) -- see the module docstring."""
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    cmd = ["soldr", "wheel"]
    if args.profile == "release":
        cmd.append("--release")
    cmd += ["--target", args.target, "--out", str(out_dir)]
    print(f"+ {' '.join(cmd)}", flush=True)
    rc = subprocess.run(cmd, cwd=ROOT, check=False).returncode
    if rc != 0:
        return rc
    wheels = sorted(out_dir.glob("*.whl"))
    if not wheels:
        print(
            f"ci/platform_build.py: soldr wheel produced no .whl in {out_dir}",
            file=sys.stderr,
        )
        return 1
    wheel_path = wheels[-1]
    print(f"wheel: {wheel_path}")
    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            fh.write(f"wheel-path={wheel_path}\n")
    return 0


# ---------------------------------------------------------------------------
# stage: turn the build-json artifacts + wheel into a self-describing,
# downloadable directory for platform-run (round 3, deliverable 3).
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class StagedBinary:
    """One declared test binary, staged for `platform-run` to execute
    directly. `name` matches ci.toml `[rust.tests].binaries`' naming
    scheme exactly (`ci_lint.runtime.tests_size._declared_name`'s scheme,
    reimplemented here rather than imported — see the module docstring on
    why `ci/*.py` treats `ci_lint` as an external pinned CLI tool, never a
    library import)."""

    name: str
    staged_path: str  # relative to the stage dir


def _declared_name(package: str, kinds: list[str], target_name: str) -> str | None:
    if "lib" in kinds:
        return f"{package}:lib"
    if "bin" in kinds:
        return f"{package}:bin:{target_name}"
    if "test" in kinds:
        return f"{package}:test:{target_name}"
    return None


def _iter_compiler_artifacts(artifacts_path: Path):
    for raw_line in artifacts_path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line:
            continue
        try:
            doc = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(doc, dict) or doc.get("reason") != "compiler-artifact":
            continue
        yield doc


def _package_name(package_id: object) -> str:
    """Same two package-id shapes `ci_lint.cargo_messages._parse_package_id`
    handles (legacy `"name version (source)"`, and the `#name@version`/
    bare-fragment SPEC forms) — see that module for the full rationale;
    reimplemented minimally here (name only, this script never needs the
    version) for the same "no library import" reason as `_declared_name`."""
    if not isinstance(package_id, str):
        return ""
    if " " in package_id:
        return package_id.split(" ", 1)[0]
    if "#" in package_id:
        head, _, tail = package_id.rpartition("#")
        if "@" in tail:
            return tail.rsplit("@", 1)[0]
        return head.rstrip("/").rsplit("/", 1)[-1] if head else ""
    return package_id


def _make_executable(path: Path) -> None:
    mode = path.stat().st_mode
    path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def cmd_stage(args: argparse.Namespace) -> int:  # noqa: C901
    stage_dir = Path(args.out)
    bin_dir = stage_dir / "bin"
    cli_dir = stage_dir / "cli"
    wheel_dir = stage_dir / "wheel"
    for d in (bin_dir, cli_dir, wheel_dir):
        d.mkdir(parents=True, exist_ok=True)

    staged: list[StagedBinary] = []
    for doc in _iter_compiler_artifacts(Path(args.artifacts)):
        profile = doc.get("profile")
        if not isinstance(profile, dict) or profile.get("test") is not True:
            continue
        executable = doc.get("executable")
        if not isinstance(executable, str) or not executable:
            continue
        target = doc.get("target")
        kinds = target.get("kind") if isinstance(target, dict) else None
        target_name = target.get("name") if isinstance(target, dict) else None
        if not isinstance(kinds, list) or not isinstance(target_name, str):
            continue
        name = _declared_name(_package_name(doc.get("package_id")), kinds, target_name)
        if name is None:
            continue
        src = Path(executable)
        if not src.is_file():
            print(
                f"ci/platform_build.py: recorded executable missing on disk: {src}",
                file=sys.stderr,
            )
            return 1
        # Filesystem-safe staged filename -- the declared name (e.g.
        # "ghd-cli:test:cli") contains ':' and platform-run maps
        # back to it purely through manifest.json, never by parsing the
        # staged filename itself.
        staged_name = name.replace(":", "__") + (".exe" if src.suffix == ".exe" else "")
        dest = bin_dir / staged_name
        shutil.copy2(src, dest)
        _make_executable(dest)
        staged.append(StagedBinary(name=name, staged_path=f"bin/{staged_name}"))

    if not staged:
        print(
            f"ci/platform_build.py: no declared test binaries found in {args.artifacts}",
            file=sys.stderr,
        )
        return 1

    wheels = sorted(Path(args.wheel_dir).glob("*.whl"))
    if not wheels:
        print(
            f"ci/platform_build.py: no wheel found in {args.wheel_dir}", file=sys.stderr
        )
        return 1
    wheel_src = wheels[-1]
    wheel_dest = wheel_dir / wheel_src.name
    shutil.copy2(wheel_src, wheel_dest)

    # Extract the bundled native CLI straight out of the wheel we are
    # about to ship, rather than compiling it a second time -- proves the
    # SAME artifact that installs is the one platform-run executes
    # directly for the CARGO_BIN_EXE_ghd-cli hookup (PKG-003).
    cli_relpath: str | None = None
    with zipfile.ZipFile(wheel_dest) as zf:
        candidates = [
            n
            for n in zf.namelist()
            if ".data/scripts/" in n and Path(n).name.startswith(args.cli_name)
        ]
        if not candidates:
            print(
                f"ci/platform_build.py: no '{args.cli_name}[.exe]' found in {wheel_dest.name}'s "
                "<dist>.data/scripts/ -- bundle-bins did not stage the CLI into this wheel "
                "(PKG-003)",
                file=sys.stderr,
            )
            return 1
        member = candidates[0]
        cli_filename = Path(member).name
        cli_dest = cli_dir / cli_filename
        with zf.open(member) as src_fh, cli_dest.open("wb") as dst_fh:
            shutil.copyfileobj(src_fh, dst_fh)
        _make_executable(cli_dest)
        cli_relpath = f"cli/{cli_filename}"

    manifest = {
        "lane_id": args.lane_id,
        "target": args.target,
        "binaries": [
            {"name": s.name, "file": s.staged_path}
            for s in sorted(staged, key=lambda s: s.name)
        ],
        "cli_binary": cli_relpath,
        "wheel": f"wheel/{wheel_dest.name}",
    }
    (stage_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2), encoding="utf-8"
    )

    print(
        f"staged {len(staged)} test binaries + cli + wheel for lane '{args.lane_id}' ({args.target}) -> {stage_dir}"
    )
    for b in manifest["binaries"]:
        print(f"  {b['name']:<32} {b['file']}")
    print(f"  cli_binary                      {cli_relpath}")
    print(f"  wheel                           {manifest['wheel']}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("build-json")
    p.add_argument("--target", required=True)
    p.add_argument("--profile", choices=["dev", "release"], default="dev")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_build_json)

    p = sub.add_parser("wheel-build")
    p.add_argument("--target", required=True)
    p.add_argument("--profile", choices=["dev", "release"], default="dev")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_wheel_build)

    p = sub.add_parser("stage")
    p.add_argument("--lane-id", required=True)
    p.add_argument("--target", required=True)
    p.add_argument("--artifacts", required=True)
    p.add_argument("--wheel-dir", required=True)
    p.add_argument("--cli-name", default="ghd")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_stage)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
