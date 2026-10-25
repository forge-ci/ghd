"""Release-lane orchestration for the `linux-x64` leg (zackees/ci.yml#6
round 5, deliverable 2): the ONE platform the `platform-build`/
`platform-run` matrix never covers (`plan.platform_lanes` excludes
`[flow.pr]`'s own default fast-lane platform -- normally just
`linux-x64` -- by construction; round 5's brief calls this out as a
"decide and record" choice). **Decision**: build linux-x64's
release-profile wheel + sdist in its own dedicated job
(`release-linux-x64` in `.github/workflows/ci.yml`) rather than teaching
`ci_lint`'s planner to special-case the release flow's platform-lane set
-- that would touch the pinned `ci_lint` package (`zackees/ci.yml`,
out of this round's repo scope), where a template-repo script is
sufficient and keeps the change entirely local.

Also holds `collect-wheels`: merging `platform-build`'s staged
per-platform wheels (each nested at `<staged-dir>/platform-<id>/wheel/
*.whl`, round 3's `manifest.json` layout, unchanged here) into one flat
`dist/` directory alongside the `linux-x64` sdist+wheel, for
`release-verify`'s `ci-lint release verify --dist dist` (issue #6 §3,
`PKG-006`).

Same rules as every other `ci/*.py` orchestration module (`ci/fast.py`,
`ci/platform_build.py`): never invokes bare `cargo`/`rustc`/`maturin`/
`pip` (`RUST-001`/`PKG-003`/`TOOL-001`) -- `soldr`/`uv` only. Does not
import any other `ci/*.py` module (self-contained, stdlib-only, matching
the existing convention recorded in `ci/README.md`).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _run(cmd: list[str]) -> int:
    print(f"+ {' '.join(cmd)}", flush=True)
    return subprocess.run(cmd, cwd=ROOT, check=False).returncode


def _run_isolated_soldr(cmd: list[str]) -> int:
    """See `ci/fast.py::_run_isolated_soldr` -- identical reasoning (two
    independent soldr broker roots can't share one claim), duplicated
    rather than imported so this module stays a self-contained,
    stdlib-only script like every other `ci/*.py` orchestration module in
    this repo."""
    env = {
        k: v for k, v in os.environ.items() if not k.startswith(("SOLDR_", "ZCCACHE_"))
    }
    print(
        f"+ {' '.join(cmd)}  (SOLDR_*/ZCCACHE_* stripped -- see docstring)", flush=True
    )
    return subprocess.run(cmd, cwd=ROOT, env=env, check=False).returncode


def _make_executable(path: Path) -> None:
    mode = path.stat().st_mode
    path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


# ---------------------------------------------------------------------------
# build: sdist (Soldr PEP 517 backend via `uv build --sdist`) + the
# release-profile wheel via `soldr wheel --release [--target <triple>]`
# (zackees/ci.yml#17/#19, ghd#37/#39).
#
# `soldr wheel --release` builds every `*-linux-gnu` wheel against soldr's
# catalogue glibc-2.17 toolchain, host target included (zackees/soldr#3432),
# and since soldr 0.9.27 bundles `[tool.soldr.pep517] bundle-bins` into it
# (zackees/soldr#3468). `sdist-smoke` below keeps PKG-004's "a wheel builds
# FROM the sdist through the PEP 517 frontend" proof for the shipped sdist.
# ---------------------------------------------------------------------------


def _write_outputs(**values: Path) -> None:
    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            for key, value in values.items():
                fh.write(f"{key.replace('_', '-')}={value}\n")


def cmd_build(args: argparse.Namespace) -> int:
    """`--target <triple>` is optional: `release-linux-x64` builds the
    host target (`soldr wheel --release` already enforces glibc 2.17 on
    it); the musl lanes pass their cross target. `glibc-check` (below)
    still proves the floor from the staged wheel's own bytes."""
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    rc = _run_isolated_soldr(["uv", "build", "--sdist", "--out-dir", str(out_dir)])
    if rc != 0:
        return rc
    cmd = ["soldr", "wheel", "--release"]
    if args.target:
        cmd += ["--target", args.target]
    rc = _run([*cmd, "--out", str(out_dir)])
    if rc != 0:
        return rc
    wheels = sorted(out_dir.glob("*.whl"))
    sdists = sorted(out_dir.glob("*.tar.gz"))
    if not wheels:
        print(
            f"ci/release.py: soldr wheel produced no .whl in {out_dir}", file=sys.stderr
        )
        return 1
    if not sdists:
        print(
            f"ci/release.py: uv build produced no sdist (.tar.gz) in {out_dir}",
            file=sys.stderr,
        )
        return 1
    print(f"wheel: {wheels[-1]}")
    print(f"sdist: {sdists[-1]}")
    _write_outputs(wheel_path=wheels[-1], sdist_path=sdists[-1])
    return 0


def cmd_sdist_smoke(args: argparse.Namespace) -> int:
    """Build a wheel FROM the shipped sdist through the Soldr PEP 517
    backend (`uv build --wheel <sdist>`) and require the bundled CLI in
    it -- the sdist is what a platform without a published wheel builds."""
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    rc = _run_isolated_soldr(
        ["uv", "build", "--wheel", args.sdist, "--out-dir", str(out_dir)]
    )
    if rc != 0:
        return rc
    wheels = sorted(out_dir.glob("*.whl"))
    if not wheels:
        print(f"ci/release.py: sdist produced no .whl in {out_dir}", file=sys.stderr)
        return 1
    with zipfile.ZipFile(wheels[-1]) as archive:
        scripts = [
            n for n in archive.namelist() if f".data/scripts/{args.cli_name}" in n
        ]
    if not scripts:
        print(
            f"ci/release.py: {wheels[-1].name} (built from the sdist) has no "
            f".data/scripts/{args.cli_name}",
            file=sys.stderr,
        )
        return 1
    print(f"sdist-smoke: {wheels[-1].name} contains {scripts[0]}")
    return 0


# ---------------------------------------------------------------------------
# glibc-check: proves the manylinux_2_17 cross sysroot did its job, from
# the staged wheel's OWN bytes -- not just the filename tag a build could
# get wrong (round-5 orchestrator correction: "inspect the bundled CLI/
# extension's maximum required GLIBC symbol version ... ≤ 2.17 -- record
# the value"). Extracts the bundled native CLI (`*.data/scripts/<cli>`)
# and the PyO3 extension (`*/_native*.so`) from the wheel, runs `readelf
# -V` (falling back to `objdump -T` if `readelf` isn't on PATH -- both
# are standard `binutils`, never a banned TOOL-001 name) against each
# extracted ELF, and parses every `GLIBC_X.Y` version-need string out of
# the text output. The MAXIMUM (X, Y) found across both files is the
# real floor this wheel needs; `--max-glibc` is ci.toml's declared one.
# Pure verification -- never invokes cargo/soldr/uv.
# ---------------------------------------------------------------------------


def _extract_elf_members(wheel_path: Path, out_dir: Path) -> list[tuple[str, Path]]:
    """Pull the bundled native CLI and the PyO3 extension out of the
    wheel into `out_dir`, returning `(member_name, extracted_path)` for
    each ELF found (skips anything that clearly isn't one, e.g. a `.pyi`
    stub or `RECORD`)."""
    extracted: list[tuple[str, Path]] = []
    with zipfile.ZipFile(wheel_path) as zf:
        for member in zf.namelist():
            is_bundled_cli = ".data/scripts/" in member
            is_native_ext = member.endswith(".so") and "_native" in member
            if not (is_bundled_cli or is_native_ext):
                continue
            dest = out_dir / Path(member).name
            with zf.open(member) as src, dest.open("wb") as dst:
                shutil.copyfileobj(src, dst)
            extracted.append((member, dest))
    return extracted


def _max_glibc_version(elf_path: Path) -> tuple[int, int] | None:
    """The highest `GLIBC_X.Y` version-need string found in `elf_path`'s
    `.gnu.version_r` (verneed) section, via `readelf -V` (or `objdump -T`
    if `readelf` is missing). `None` means the binary references no
    versioned GLIBC symbol at all (e.g. fully static, or no libc
    dependency) -- reported, never silently treated as "0.0 floor met"."""
    tool_argv_candidates = [
        ["readelf", "-V", str(elf_path)],
        ["objdump", "-T", str(elf_path)],
    ]
    output: str | None = None
    tool_used: str | None = None
    for argv in tool_argv_candidates:
        if shutil.which(argv[0]) is None:
            continue
        proc = subprocess.run(argv, check=False, capture_output=True, text=True)
        if proc.returncode == 0:
            output = proc.stdout
            tool_used = argv[0]
            break
    if output is None:
        raise RuntimeError(
            f"neither 'readelf' nor 'objdump' produced usable output for {elf_path} "
            "(is binutils installed on this runner?)"
        )
    versions = {
        (int(m.group(1)), int(m.group(2)))
        for m in re.finditer(r"GLIBC_(\d+)\.(\d+)", output)
    }
    print(
        f"ci/release.py: {elf_path.name}: {tool_used} found GLIBC versions {sorted(versions)}"
    )
    return max(versions) if versions else None


def cmd_glibc_check(args: argparse.Namespace) -> int:
    wheel_path = Path(args.wheel)
    if not wheel_path.is_file():
        print(f"ci/release.py: {wheel_path} is not a file", file=sys.stderr)
        return 2
    try:
        max_major, max_minor = (int(p) for p in args.max_glibc.split("."))
    except ValueError:
        print(
            f"ci/release.py: --max-glibc {args.max_glibc!r} is not 'X.Y'",
            file=sys.stderr,
        )
        return 2

    with tempfile.TemporaryDirectory() as tmp:
        tmp_dir = Path(tmp)
        members = _extract_elf_members(wheel_path, tmp_dir)
        if not members:
            print(
                f"ci/release.py: glibc-check: no ELF members found in {wheel_path}",
                file=sys.stderr,
            )
            return 2

        try:
            found = [(name, _max_glibc_version(path)) for name, path in members]
        except RuntimeError as exc:
            print(f"ci/release.py: glibc-check: {exc}", file=sys.stderr)
            return 2

    overall = [v for _name, v in found if v is not None]
    if not overall:
        print(
            "ci/release.py: glibc-check: no GLIBC_X.Y version-need strings found in any "
            "bundled ELF -- needs_review, not a pass (unexpected for a glibc-linked binary)"
        )
        return 2

    worst_name, worst = max(
        ((name, v) for name, v in found if v is not None), key=lambda t: t[1]
    )
    floor = (max_major, max_minor)
    passed = worst <= floor
    print(
        f"ci/release.py: glibc-check: max required GLIBC_{worst[0]}.{worst[1]} "
        f"(from {worst_name}), floor manylinux_{max_major}_{max_minor} -> "
        f"{'PASS' if passed else 'FAIL'}"
    )
    return 0 if passed else 1


# ---------------------------------------------------------------------------
# smoke: clean-venv native install smoke of the just-built linux-x64
# wheel, writing the SAME smoke-results/<id>.json shape
# `ci/platform_run.py`'s `wheel-install --smoke-out` writes, so
# `ci-lint release verify --smoke smoke-results` sees one uniform format
# for every platform (issue #6 §3, PKG-006).
# ---------------------------------------------------------------------------


def cmd_smoke(args: argparse.Namespace) -> int:
    wheels = sorted(Path(args.wheel_dir).glob("*.whl"))
    if not wheels:
        print(f"ci/release.py: no wheel found in {args.wheel_dir}", file=sys.stderr)
        return 1
    wheel_path = wheels[-1]

    venv_dir = Path(args.venv)
    rc = subprocess.run(
        ["uv", "venv", str(venv_dir), "--python", args.python], cwd=ROOT, check=False
    ).returncode
    if rc != 0:
        return rc
    venv_python = (
        venv_dir / "bin" / "python3"
    )  # linux-x64 only -- this job is ubuntu-24.04-only
    rc = subprocess.run(
        ["uv", "pip", "install", "--python", str(venv_python), str(wheel_path)],
        cwd=ROOT,
        check=False,
    ).returncode

    passed = False
    detail = "install failed"
    if rc == 0:
        cli_path = venv_dir / "bin" / args.cli_name
        if cli_path.is_file():
            _make_executable(cli_path)
            proc = subprocess.run([str(cli_path), "--version"], cwd=ROOT, check=False)
            passed = proc.returncode == 0
            detail = f"{args.cli_name} --version exit {proc.returncode}"
        else:
            detail = f"installed wheel has no {cli_path}"

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(
        json.dumps(
            {
                "platform": args.platform_id,
                "wheel": wheel_path.name,
                "passed": passed,
                "detail": detail,
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    print(f"smoke ({args.platform_id}): passed={passed} ({detail}) -> {out_path}")
    return 0 if passed else 1


# ---------------------------------------------------------------------------
# collect-wheels: pull platform-build's staged `wheel/*.whl` out of each
# downloaded `platform-<id>` artifact directory into one flat `dist/`
# alongside the linux-x64 sdist+wheel `release-verify` already has.
# ---------------------------------------------------------------------------


def cmd_collect_wheels(args: argparse.Namespace) -> int:
    staged_dir = Path(args.staged_dir)
    dist_dir = Path(args.dist_dir)
    dist_dir.mkdir(parents=True, exist_ok=True)

    if not staged_dir.is_dir():
        print(f"ci/release.py: {staged_dir} is not a directory", file=sys.stderr)
        return 1

    found = sorted(staged_dir.glob("*/wheel/*.whl"))
    if not found:
        print(f"ci/release.py: no '*/wheel/*.whl' under {staged_dir}", file=sys.stderr)
        return 1
    for wheel_path in found:
        dest = dist_dir / wheel_path.name
        shutil.copy2(wheel_path, dest)
        print(f"collected {wheel_path} -> {dest}")
    return 0


# ---------------------------------------------------------------------------
# mock-publish / mock-readback: zackees/ci.yml#8/#74 (REL-003) -- a
# non-publishing dry run must exercise the mock publisher's destination
# *read* path, not just its write path. mimalloc-pprof#562: a real
# publisher's `GET .../releases/tags/{tag}` 404s for a draft release (only
# the paged list endpoint includes drafts), and a dry run that only WRITES
# never catches that class of bug.
#
# `mock-publish` writes each staged artifact into a throwaway "mock
# registry" directory -- standing in for the real publisher's destination
# (a PyPI/crates.io/GitHub Releases draft in the real pilot). `mock-
# readback` then READS each artifact back FROM that registry directory
# (never from the original `dist/` copy) and writes one
# `readback/<artifact>.json` record ci-lint's own `release verify
# --readback` (REL-003) checks -- `{"path", "sha256", "read_back": true}`,
# with the sha256 computed from the bytes actually read back.
# ---------------------------------------------------------------------------


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def cmd_mock_publish(args: argparse.Namespace) -> int:
    dist_dir = Path(args.dist)
    registry_dir = Path(args.registry)
    registry_dir.mkdir(parents=True, exist_ok=True)
    if not dist_dir.is_dir():
        print(f"ci/release.py: --dist {dist_dir} is not a directory", file=sys.stderr)
        return 1
    staged = [
        p
        for p in sorted(dist_dir.iterdir())
        if p.suffix == ".whl" or p.name.endswith(".tar.gz")
    ]
    if not staged:
        print(
            f"ci/release.py: mock-publish: no wheel/sdist found in {dist_dir}",
            file=sys.stderr,
        )
        return 1
    for artifact in staged:
        dest = registry_dir / artifact.name
        shutil.copy2(artifact, dest)
        print(f"mock-publish: wrote {artifact.name} -> {dest}")
    return 0


def cmd_mock_readback(args: argparse.Namespace) -> int:
    dist_dir = Path(args.dist)
    registry_dir = Path(args.registry)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    if not dist_dir.is_dir():
        print(f"ci/release.py: --dist {dist_dir} is not a directory", file=sys.stderr)
        return 1
    staged = [
        p
        for p in sorted(dist_dir.iterdir())
        if p.suffix == ".whl" or p.name.endswith(".tar.gz")
    ]
    rc = 0
    for artifact in staged:
        registry_copy = registry_dir / artifact.name
        if not registry_copy.is_file():
            print(
                f"ci/release.py: mock-readback: '{artifact.name}' was never written to the mock "
                f"registry ({registry_copy}) -- the destination read path finds nothing",
                file=sys.stderr,
            )
            rc = 1
            continue
        # The whole point: hash the bytes actually READ BACK from the mock
        # destination, never the local dist/ copy -- a write bug that
        # corrupts/loses bytes at the destination must show up here.
        sha256 = _sha256_file(registry_copy)
        record_path = out_dir / f"{artifact.name}.json"
        record_path.write_text(
            json.dumps(
                {"path": artifact.name, "sha256": sha256, "read_back": True}, indent=2
            ),
            encoding="utf-8",
        )
        print(
            f"mock-readback: read {artifact.name} back from {registry_copy} -> {record_path}"
        )
    return rc


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("build")
    p.add_argument("--target", default=None)
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_build)

    p = sub.add_parser("sdist-smoke")
    p.add_argument("--sdist", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--cli-name", default="ghd")
    p.set_defaults(func=cmd_sdist_smoke)

    p = sub.add_parser("smoke")
    p.add_argument("--wheel-dir", required=True)
    p.add_argument("--platform-id", required=True)
    p.add_argument("--venv", required=True)
    p.add_argument("--python", default="3.11")
    p.add_argument("--cli-name", default="ghd")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_smoke)

    p = sub.add_parser("collect-wheels")
    p.add_argument("--staged-dir", required=True)
    p.add_argument("--dist-dir", required=True)
    p.set_defaults(func=cmd_collect_wheels)

    p = sub.add_parser("glibc-check")
    p.add_argument("--wheel", required=True)
    p.add_argument("--max-glibc", required=True, help="e.g. 2.17")
    p.set_defaults(func=cmd_glibc_check)

    p = sub.add_parser("mock-publish")
    p.add_argument("--dist", required=True)
    p.add_argument("--registry", required=True)
    p.set_defaults(func=cmd_mock_publish)

    p = sub.add_parser("mock-readback")
    p.add_argument("--dist", required=True)
    p.add_argument("--registry", required=True)
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_mock_readback)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
