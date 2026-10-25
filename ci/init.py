"""`[ci-test-init]` suite orchestration: build + wheel-smoke a `--target`
tree (an `ci/instantiate.py`-generated copy), from zero.

zackees/ci.yml#6 round-4B-followup: the `init` job runs with NO cache
restore (its own `Setup soldr` step passes `cache: "false"`; its
`Install uv` step passes `enable-cache: false` -- see ci.toml [suites]
.init and the `init` job in .github/workflows/ci.yml) against a freshly
instantiated copy of this repo, proving the template bootstraps from a
cold, cache-free clone -- not just that caches happen to mask a broken
step. Each workflow step is one line calling a subcommand here
(GEN-005), mirroring `ci/fast.py`'s shape but every subcommand takes
`--target <dir>` (the instantiated tree) instead of operating on this
checkout's own root.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

import tomllib


def _run(cmd: list[str], *, cwd: Path) -> int:
    print(f"+ {' '.join(cmd)}  (cwd={cwd})", flush=True)
    return subprocess.run(cmd, cwd=cwd, check=False).returncode


def _run_isolated_soldr(cmd: list[str], *, cwd: Path) -> int:
    """Same reasoning as `ci/fast.py::_run_isolated_soldr`: a nested PEP
    517 build resolves its OWN `soldr` (the instantiated tree's own
    `pyproject.toml` `requires = ["soldr>=0.9.26"]`), which must not
    contend with the outer job's own soldr broker."""
    env = {
        k: v for k, v in os.environ.items() if not k.startswith(("SOLDR_", "ZCCACHE_"))
    }
    print(f"+ {' '.join(cmd)}  (cwd={cwd}, SOLDR_*/ZCCACHE_* stripped)", flush=True)
    return subprocess.run(cmd, cwd=cwd, env=env, check=False).returncode


def cmd_build(args: argparse.Namespace) -> int:
    """`soldr cargo build --workspace --locked` against the instantiated
    tree -- proves it compiles from zero, no `--message-format=json`
    capture (the `init` job does not run `ci_lint units`/`tests size`:
    those check THIS repo's own `[rust.tests].binaries` declarations,
    which describe crate names the instantiated tree deliberately no
    longer has)."""
    target = Path(args.target)
    return _run(["soldr", "cargo", "build", "--workspace", "--locked"], cwd=target)


def cmd_wheel_build(args: argparse.Namespace) -> int:
    target = Path(args.target)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    rc = _run_isolated_soldr(
        ["uv", "build", "--wheel", "--out-dir", str(out_dir)], cwd=target
    )
    if rc != 0:
        return rc
    wheels = sorted(out_dir.glob("*.whl"))
    if not wheels:
        print(f"ci/init.py: uv build produced no .whl in {out_dir}", file=sys.stderr)
        return 1
    print(f"wheel: {wheels[-1]}")
    return 0


def cmd_wheel_install(args: argparse.Namespace) -> int:
    """Clean-venv install of the built wheel (mirrors `ci/fast.py
    cmd_wheel_install`, parameterized on `--target`'s own dist dir)."""
    wheels = sorted(Path(args.wheel_dir).glob("*.whl"))
    if not wheels:
        print(f"ci/init.py: no wheel found in {args.wheel_dir}", file=sys.stderr)
        return 1
    wheel = wheels[-1]
    venv_dir = Path(args.venv)
    rc = _run(
        ["uv", "venv", str(venv_dir), "--python", args.python], cwd=Path(args.target)
    )
    if rc != 0:
        return rc
    python_bin = venv_dir / "bin" / "python3"
    return _run(
        ["uv", "pip", "install", "--python", str(python_bin), str(wheel)],
        cwd=Path(args.target),
    )


def cmd_wheel_smoke(args: argparse.Namespace) -> int:
    """The native CLI on PATH runs, and the PyO3 extension imports --
    same shape `ci_lint wheel installed` checks for the real tree, done
    directly here (not via `ci_lint`) since the instantiated tree's
    package/CLI name is only known at instantiate-time, read back from
    its own `pyproject.toml`/`ci.toml` rather than hardcoded."""
    target = Path(args.target)
    venv_dir = Path(args.venv)
    pyproject = tomllib.loads((target / "pyproject.toml").read_text(encoding="utf-8"))
    ci_toml = tomllib.loads((target / "ci.toml").read_text(encoding="utf-8"))
    pkg = pyproject["project"]["name"].replace("-", "_")
    cli = ci_toml["python"]["cli"]["name"]
    python_bin = venv_dir / "bin" / "python3"
    cli_bin = venv_dir / "bin" / cli
    rc = _run([str(cli_bin), "--version"], cwd=target)
    if rc != 0:
        return rc
    return _run(
        [
            str(python_bin),
            "-c",
            f"import {pkg}; import {pkg}._native as n; print(n.__file__)",
        ],
        cwd=target,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("build")
    p.add_argument("--target", required=True)
    p.set_defaults(func=cmd_build)

    p = sub.add_parser("wheel-build")
    p.add_argument("--target", required=True)
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_wheel_build)

    p = sub.add_parser("wheel-install")
    p.add_argument("--target", required=True)
    p.add_argument("--wheel-dir", required=True)
    p.add_argument("--venv", required=True)
    p.add_argument("--python", default="3.11")
    p.set_defaults(func=cmd_wheel_install)

    p = sub.add_parser("wheel-smoke")
    p.add_argument("--target", required=True)
    p.add_argument("--venv", required=True)
    p.set_defaults(func=cmd_wheel_smoke)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
