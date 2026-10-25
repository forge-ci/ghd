"""`release-musl-smoke` (ci.yml#43): native install smoke of a just-built
musllinux wheel, run INSIDE a digest-pinned `python:3.13-alpine` (musl
libc) container -- the only environment that can actually load a
musllinux wheel's native extension (a glibc-linked CPython cannot dlopen
a musl-linked PyO3 extension, and pip on a glibc/manylinux host does not
even consider a musllinux-tagged wheel installable in the first place).

Deliberately does NOT use `soldr`/`uv` (banned bare-tool rule, ci.toml
[allow].tools) -- it doesn't need to: Alpine's `python:3.13-alpine` image
ships neither, so this script uses only the CPython stdlib (`venv`,
`ensurepip`-bundled `pip`) exactly like `ci/release.py::cmd_smoke` does
for linux-x64, minus the soldr/uv build step (the wheel is already built,
by `release-musl-build`; this only installs and smoke-tests it). Writes
the SAME smoke-results/<id>.json shape every other platform's smoke step
writes, so `ci-lint release verify --smoke smoke-results` (PKG-006) sees
one uniform format for every platform.
"""

from __future__ import annotations

import argparse
import json
import stat
import subprocess
import sys
from pathlib import Path


def _make_executable(path: Path) -> None:
    mode = path.stat().st_mode
    path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheel-dir", required=True)
    parser.add_argument("--platform-id", required=True)
    parser.add_argument("--venv", required=True)
    parser.add_argument("--cli-name", default="ghd")
    parser.add_argument("--out", required=True)
    args = parser.parse_args(argv)

    wheels = sorted(
        p
        for p in Path(args.wheel_dir).glob("*.whl")
        if args.platform_id in p.name or "musllinux" in p.name
    )
    if not wheels:
        # Fall back to any wheel in the dir (single-wheel dist, the
        # normal case: `release-musl-build` stages exactly one wheel +
        # one sdist per platform id).
        wheels = sorted(Path(args.wheel_dir).glob("*.whl"))
    if not wheels:
        print(f"ci/musl_smoke.py: no wheel found in {args.wheel_dir}", file=sys.stderr)
        return 1
    wheel_path = wheels[-1]

    venv_dir = Path(args.venv)
    rc = subprocess.run(
        [sys.executable, "-m", "venv", str(venv_dir)], check=False
    ).returncode
    if rc != 0:
        return rc
    venv_python = venv_dir / "bin" / "python3"
    rc = subprocess.run(
        [
            str(venv_python),
            "-m",
            "pip",
            "install",
            "--no-index",
            "--no-build-isolation",
            str(wheel_path),
        ],
        check=False,
    ).returncode

    passed = False
    detail = "install failed"
    if rc == 0:
        cli_path = venv_dir / "bin" / args.cli_name
        if cli_path.is_file():
            _make_executable(cli_path)
            proc = subprocess.run([str(cli_path), "--version"], check=False)
            passed = proc.returncode == 0
            detail = f"{args.cli_name} --version exit {proc.returncode} (musllinux, Alpine container)"
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
    print(f"musl smoke ({args.platform_id}): passed={passed} ({detail}) -> {out_path}")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
