"""Platform-run lane orchestration: execute one platform's pre-built test
binaries and smoke the wheel, with NO Rust toolchain on this runner
(zackees/ci.yml#6 round 3, deliverable 4). Everything Rust-shaped was
already built on Linux by the matching `platform-build` leg
(`ci/platform_build.py`) and downloaded here as a plain artifact.

Each `.github/workflows/ci.yml` `platform-run` matrix leg's steps are one
line each calling `python3 ci/platform_run.py <subcommand>` (CLAUDE.md
rule 6 / `GEN-005`); this module holds the logic.

Never invokes `soldr`/`cargo`/`rustc`/`maturin` — there is nothing to
compile here. `uv` is the only build-adjacent tool this module calls
(venv creation + wheel install), matching `ci/fast.py`'s clean-venv smoke
step exactly, just against a downloaded wheel instead of a freshly-built
one.
"""

from __future__ import annotations

import argparse
import json
import os
import stat
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# ci.toml [allow].platform-code does not list ci/**, so this script (like
# every other ci/*.py) reaches host-specific behavior only through the
# Python platform facade -- never a bare os.name/sys.platform check (
# LAYOUT-001). Importable without a build: see
# src/ghd/platforms/README.md ("zero runtime
# dependencies").
sys.path.insert(0, str(ROOT / "src"))
# pylint: disable-next=wrong-import-position
from ghd.platforms import is_windows  # noqa: E402

# The one declared test binary that needs the actual compiled CLI at
# RUNTIME (crates/ghd-cli/tests/cli/main.rs reads
# CARGO_BIN_EXE_ghd via std::env::var, not the env!/option_env!
# compile-time macros -- see that file's own docstring for why). Every
# other declared binary ignores this env var; setting it unconditionally
# for all of them is harmless.
CARGO_BIN_EXE_ENV = "CARGO_BIN_EXE_ghd"


def _venv_python(venv_dir: Path) -> Path:
    """The venv's interpreter path, using the platform facade
    (`is_windows()`) instead of a bare `os.name` check -- see the
    LAYOUT-001 note above."""
    if is_windows():
        return venv_dir / "Scripts" / "python.exe"
    return venv_dir / "bin" / "python3"


def _uv_python_spec(python_version: str, target: object) -> str:
    """uv's own Python download, left to auto-detect the host on a
    `windows-11-arm` GitHub-hosted runner, resolves to a Windows
    **x86_64** build run under WoA emulation (uv's documented behavior:
    python-build-standalone has no native Windows/aarch64 CPython, so
    x64-under-emulation is the intended default there) -- found via run
    36499249542's platform-run (windows-arm64) leg:
    `uv pip install`'s own `error: Failed to determine installation
    plan ... the wheel is compatible with Windows (win_arm64), but
    you're on Windows (win_amd64)`. There is no way to install a
    win_arm64-tagged wheel into an x64-emulated interpreter, so for this
    ONE target this must request uv's Windows ARM64 build explicitly
    (`<impl>-<version>-<os>-<arch>-<libc>`, see
    https://docs.astral.sh/uv/concepts/python-versions/); every other
    target keeps the plain version string uv already resolves
    correctly."""
    if isinstance(target, str) and target == "aarch64-pc-windows-msvc":
        return f"cpython-{python_version}-windows-aarch64-none"
    return python_version


def _make_executable(path: Path) -> None:
    """`actions/upload-artifact`/`download-artifact`'s zip-based transport
    does not reliably preserve the Unix execute bit `ci/platform_build.py`
    set before uploading -- found via run 36498870719's platform-run
    (macos-x64/macos-arm64) legs: `PermissionError: [Errno 13] Permission
    denied` on the downloaded, un-executable `ghd-cli__test__cli`.
    Re-applied here, after download, on every staged binary before it is
    ever spawned. A harmless no-op on Windows (NTFS has no Unix execute
    bit; `.exe` is inherently runnable -- os.chmod there only touches the
    read-only flag)."""
    mode = path.stat().st_mode
    path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def _load_manifest(artifact_dir: Path) -> dict[str, object]:
    manifest_path = artifact_dir / "manifest.json"
    if not manifest_path.is_file():
        raise SystemExit(
            f"ci/platform_run.py: no manifest.json in {artifact_dir} (platform-build didn't stage it)"
        )
    return json.loads(manifest_path.read_text(encoding="utf-8"))


@dataclass(frozen=True)
class BinaryRunResult:
    name: str
    seconds: float
    returncode: int

    def to_json_dict(self) -> dict[str, object]:
        return {
            "name": self.name,
            "seconds": round(self.seconds, 3),
            "returncode": self.returncode,
        }


def cmd_run_tests(args: argparse.Namespace) -> int:  # noqa: C901
    """Execute every declared test binary this lane's platform-build leg
    staged, directly -- no nextest, no archive replay (soldr#3294 never
    applies here; see ci/platform_build.py's module docstring). Each
    binary is an ordinary `cargo test --no-run`-built libtest harness:
    running it with no arguments runs every `#[test]` inside it and
    exits non-zero on any failure."""
    artifact_dir = Path(args.artifact_dir)
    manifest = _load_manifest(artifact_dir)
    binaries = manifest.get("binaries")
    cli_binary = manifest.get("cli_binary")
    if not isinstance(binaries, list) or not binaries:
        print(
            f"ci/platform_run.py: manifest.json has no 'binaries' entries in {artifact_dir}",
            file=sys.stderr,
        )
        return 1

    env = dict(os.environ)
    if isinstance(cli_binary, str) and cli_binary:
        cli_path = (artifact_dir / cli_binary).resolve()
        if not cli_path.is_file():
            print(
                f"ci/platform_run.py: manifest cli_binary missing on disk: {cli_path}",
                file=sys.stderr,
            )
            return 1
        _make_executable(cli_path)
        env[CARGO_BIN_EXE_ENV] = str(cli_path)
        print(f"{CARGO_BIN_EXE_ENV}={cli_path}")

    results: list[BinaryRunResult] = []
    failed = False
    for entry in binaries:
        if not isinstance(entry, dict):
            continue
        name = entry.get("name")
        rel = entry.get("file")
        if not isinstance(name, str) or not isinstance(rel, str):
            continue
        exe = (artifact_dir / rel).resolve()
        _make_executable(exe)
        print(f"+ {exe}  # {name}", flush=True)
        start = time.monotonic()
        proc = subprocess.run([str(exe)], cwd=ROOT, env=env, check=False)
        seconds = time.monotonic() - start
        results.append(
            BinaryRunResult(name=name, seconds=seconds, returncode=proc.returncode)
        )
        print(f"  -> rc={proc.returncode} in {seconds:.2f}s")
        if proc.returncode != 0:
            failed = True

    total_seconds = sum(r.seconds for r in results)
    print(
        f"platform-run ({manifest.get('lane_id')}): {len(results)} binary(s), {total_seconds:.1f}s total"
    )
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write(
                f"\n### platform-run ({manifest.get('lane_id')}) test binaries\n\n"
            )
            fh.write("| binary | seconds | rc |\n|---|---|---|\n")
            for r in results:
                fh.write(f"| {r.name} | {r.seconds:.2f} | {r.returncode} |\n")
            fh.write(f"\n**total**: {total_seconds:.1f}s\n")
    if args.results_out:
        Path(args.results_out).write_text(
            json.dumps([asdict(r) for r in results], indent=2), encoding="utf-8"
        )

    return 1 if failed else 0


def _write_smoke_result(
    out_path: Path, platform_id: str, wheel_name: str, passed: bool, detail: str
) -> None:
    """Same `smoke-results/<id>.json` shape `ci/release.py smoke` writes
    for the `linux-x64` leg (round 5, zackees/ci.yml#6 deliverable 2) --
    one uniform format `ci-lint release verify --smoke smoke-results`
    reads for every platform."""
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(
        json.dumps(
            {
                "platform": platform_id,
                "wheel": wheel_name,
                "passed": passed,
                "detail": detail,
            },
            indent=2,
        ),
        encoding="utf-8",
    )


def cmd_wheel_install(args: argparse.Namespace) -> int:
    """Clean-venv install of the downloaded wheel with the floor Python
    (`[python].pythons[0]`) -- same shape as `ci/fast.py wheel-install`,
    against a *downloaded* wheel instead of a freshly-built one.

    `--smoke-out <path>` (round 5, optional -- always passed by
    `.github/workflows/ci.yml`, harmless when unused): after install,
    additionally runs the installed native CLI's `--version` once and
    writes a `smoke-results/<lane>.json` record. Only actually UPLOADED
    as an artifact on a release/nightly run (`needs.precheck.outputs.
    is-release == 'true'`) -- see that job's own upload step -- but
    always written so this subcommand has one code path regardless of
    which flow invoked it."""
    artifact_dir = Path(args.artifact_dir)
    manifest = _load_manifest(artifact_dir)
    wheel_rel = manifest.get("wheel")
    if not isinstance(wheel_rel, str) or not wheel_rel:
        print(
            f"ci/platform_run.py: manifest.json has no 'wheel' entry in {artifact_dir}",
            file=sys.stderr,
        )
        return 1
    wheel_path = (artifact_dir / wheel_rel).resolve()
    if not wheel_path.is_file():
        print(
            f"ci/platform_run.py: manifest wheel missing on disk: {wheel_path}",
            file=sys.stderr,
        )
        return 1

    venv_dir = Path(args.venv)
    python_spec = _uv_python_spec(args.python, manifest.get("target"))
    rc = subprocess.run(
        ["uv", "venv", str(venv_dir), "--python", python_spec], cwd=ROOT, check=False
    ).returncode
    if rc != 0:
        return rc
    venv_python = _venv_python(venv_dir)
    rc = subprocess.run(
        ["uv", "pip", "install", "--python", str(venv_python), str(wheel_path)],
        cwd=ROOT,
        check=False,
    ).returncode
    if rc != 0:
        return rc

    print(f"wheel: {wheel_path}")
    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            fh.write(f"wheel-path={wheel_path}\n")

    if args.smoke_out:
        cli_name = args.cli_name + (".exe" if is_windows() else "")
        cli_dir = venv_dir / "Scripts" if is_windows() else venv_dir / "bin"
        cli_path = cli_dir / cli_name
        passed = False
        detail = f"installed wheel has no {cli_path}"
        if cli_path.is_file():
            if not is_windows():
                _make_executable(cli_path)
            proc = subprocess.run([str(cli_path), "--version"], cwd=ROOT, check=False)
            passed = proc.returncode == 0
            detail = f"{args.cli_name} --version exit {proc.returncode}"
        lane_id = manifest.get("lane_id")
        _write_smoke_result(
            Path(args.smoke_out),
            lane_id if isinstance(lane_id, str) else "unknown",
            wheel_path.name,
            passed,
            detail,
        )
        print(f"smoke ({lane_id}): passed={passed} ({detail}) -> {args.smoke_out}")

    return 0


def cmd_integration_test(args: argparse.Namespace) -> int:
    """`ci.toml [suites].integration` on a platform-run lane: only
    invoked by a workflow step gated on `contains(matrix.lane.suites,
    'integration')`. Installs `pytest` into the ALREADY wheel-populated
    clean venv (no dev editable install here -- see
    `tests/integration/README.md`), then runs the suite with
    `-o pythonpath=` so the test can only see the installed wheel in that
    venv's site-packages, never this checkout's `src/` tree (which
    `pyproject.toml`'s `pythonpath = ["src"]` would otherwise leak in,
    since pytest still reads that file's `[tool.pytest.ini_options]` for
    any invocation made from inside this checkout)."""
    venv_dir = Path(args.venv)
    venv_python = _venv_python(venv_dir)
    rc = subprocess.run(
        ["uv", "pip", "install", "--python", str(venv_python), "pytest"],
        cwd=ROOT,
        check=False,
    ).returncode
    if rc != 0:
        return rc
    # tests/integration/test_installed_end_to_end.py does shutil.which(
    # "ghd-cli") -- it must resolve to the venv's OWN staged copy
    # (bundled into the wheel, installed here), not anything else on the
    # outer PATH. Calling the venv's python by its absolute path (as
    # everywhere else in this module does) does NOT activate the venv --
    # unlike `uv run`, which the fast lane's equivalent step uses and
    # which prepends the venv's bin/Scripts dir automatically -- so PATH
    # must be extended by hand here (found via run 36499249542's
    # platform-run (linux-arm64)/[ci-full] leg: "ghd-cli not on
    # PATH").
    venv_bin_dir = venv_python.parent
    env = dict(os.environ)
    env["PATH"] = os.pathsep.join([str(venv_bin_dir), env.get("PATH", "")])
    cmd = [
        str(venv_python),
        "-m",
        "pytest",
        "-o",
        "pythonpath=",
        "-m",
        "integration",
        "tests/integration",
    ]
    print(f"+ {' '.join(cmd)}", flush=True)
    return subprocess.run(cmd, cwd=ROOT, env=env, check=False).returncode


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("run-tests")
    p.add_argument("--artifact-dir", required=True)
    p.add_argument("--results-out", default=None)
    p.set_defaults(func=cmd_run_tests)

    p = sub.add_parser("wheel-install")
    p.add_argument("--artifact-dir", required=True)
    p.add_argument("--venv", required=True)
    p.add_argument("--python", default="3.11")
    p.add_argument("--cli-name", default="ghd")
    p.add_argument("--smoke-out", default=None)
    p.set_defaults(func=cmd_wheel_install)

    p = sub.add_parser("integration-test")
    p.add_argument("--venv", required=True)
    p.set_defaults(func=cmd_integration_test)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
