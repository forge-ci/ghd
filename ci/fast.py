"""Fast lane orchestration: linux-x64 build, unit tests, wheel smoke.

zackees/ci.yml#6 §3/§4. Each `.github/workflows/ci.yml` `fast` job step is
one line calling `python3 ci/fast.py <subcommand>` (CLAUDE.md rule 6 /
`GEN-005`); this module holds the logic so the workflow YAML stays thin.

Decisions this round made (see the PR body / worker report for evidence):
  - fmt and clippy stay separate soldr invocations (`./ci.py fmt`, `./ci.py
    clippy`), NOT folded into `soldr ci-test`'s bundled DAG. `soldr ci-test`
    also runs a host-only Dylint pass as part of that DAG; running it here
    would duplicate the dedicated `dylint` job's host pass with no shared
    cache (the two jobs run in parallel, so neither can warm the other's
    Actions cache mid-run). policy-rust.md is explicit: "Do not interpret
    the Dylint rule as permission to omit Clippy or formatting" — Clippy is
    Soldr-routed but its own required check, answering ci.yml#6's open
    question 3.
  - Build+test capture `--message-format=json` in a SEPARATE `--no-run`
    pass (`build-json`) so the file `ci_lint units`/`ci_lint tests size`
    read is pure JSON — cargo interleaves human test output with the
    compiler-message stream once tests actually execute.
  - The wheel build uses `soldr wheel` (zackees/ci.yml#17,
    ghd#37): the fleet's blessed wheel surface. Since
    soldr 0.9.27 (zackees/soldr#3468) it honours `[tool.soldr.pep517]
    bundle-bins` through the PEP 517 backend's own `_bundle_bins.py`, so
    the wheel still carries `<dist>.data/scripts/ghd-cli` (PKG-003).
    It runs the job's own soldr (no nested PEP 517 soldr), so it needs no
    `_run_isolated_soldr` env stripping. The PEP 517 backend itself is
    still exercised by `python-sync` (`uv sync`, editable install) and by
    `ci/release.py sdist-smoke` (wheel built FROM the sdist).

Never invokes bare `cargo`/`rustc`/`maturin`/`pip` (RUST-001/PKG-003) — see
`ci/gates/*.py` for the existing fmt/clippy/build/test gates this module
does not duplicate.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _run(cmd: list[str]) -> int:
    print(f"+ {' '.join(cmd)}", flush=True)
    return subprocess.run(cmd, cwd=ROOT, check=False).returncode


def _run_isolated_soldr(cmd: list[str]) -> int:
    """Run a `uv` command whose PEP 517 build environment installs its OWN
    `soldr` (pyproject.toml's `requires = ["soldr>=0.9.26"]`) -- a SEPARATE
    binary from the one `.github/actions/soldr` already started a broker
    for on PATH. Two different soldr binaries cannot share one
    broker-owned root -- confirmed on real CI (run 36489839023, job
    109155586748, and again for `uv build` in run 36492167256, job
    109163261830: "the running broker was started from a different Soldr
    image", then "soldr root ownership is busy: .../setup-soldr-soldr (no
    daemon route claim to name the owner)"). Stripping the outer job's
    `SOLDR_*`/`ZCCACHE_*` workspace-state env vars makes the nested PEP
    517 build resolve its own independent soldr session instead of
    contending for the already-claimed one. RUSTUP_HOME/CARGO_HOME/
    RUSTUP_TOOLCHAIN stay -- those select the toolchain, not a broker
    root."""
    env = {
        k: v for k, v in os.environ.items() if not k.startswith(("SOLDR_", "ZCCACHE_"))
    }
    print(
        f"+ {' '.join(cmd)}  (SOLDR_*/ZCCACHE_* stripped -- see docstring)", flush=True
    )
    return subprocess.run(cmd, cwd=ROOT, env=env, check=False).returncode


def cmd_build_json(args: argparse.Namespace) -> int:
    """Compile (no run) every declared test binary, capturing pure
    `--message-format=json` compiler-artifact records for
    `ci_lint units`/`ci_lint tests size`."""
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        "soldr",
        "cargo",
        "test",
        "--workspace",
        "--locked",
        "--no-run",
        "--message-format=json",
    ]
    print(f"+ {' '.join(cmd)} > {out_path}", flush=True)
    with out_path.open("w", encoding="utf-8") as fh:
        proc = subprocess.run(cmd, cwd=ROOT, stdout=fh, check=False)
    return proc.returncode


def cmd_rust_test(_args: argparse.Namespace) -> int:
    """Actually run the workspace's declared test binaries. Reuses the
    `build-json` step's compile output (same lockfile, same flags), so
    this is normally a near-instant re-check plus execution."""
    return _run(["soldr", "cargo", "test", "--workspace", "--locked"])


def cmd_python_sync(_args: argparse.Namespace) -> int:
    """Editable-install the project through the real Soldr PEP 517 backend
    (builds `_native` in place under `src/ghd/` and
    bundles `ghd-cli`) -- WITHOUT running any tests. Split out from
    `cmd_python_test` (below) so the `pylint` gate (GEN-004; needs
    `_native` importable for `extension-pkg-allow-list`) can depend on
    just this, unconditionally, independent of whether the `unit` suite
    is selected (ci.yml#6 round-4B suite gating: `[no-test]` skips the
    test steps below, never packaging/lint). See `_run_isolated_soldr`'s
    docstring for why the nested `uv sync` needs an isolated soldr root.
    """
    return _run_isolated_soldr(["uv", "sync", "--frozen"])


def cmd_python_test(_args: argparse.Namespace) -> int:
    """Sync (see `cmd_python_sync`), then run pytest against that
    installed artifact — not an in-place `cargo build`.

    `-m "not integration"`: the `unit` suite never runs
    `tests/integration/` (ci.toml `[suites].integration`, opt-in only —
    see `cmd_integration_test` below and `tests/integration/README.md`).
    """
    rc = _run_isolated_soldr(["uv", "sync", "--frozen"])
    if rc != 0:
        return rc
    return _run(["uv", "run", "--no-sync", "pytest", "-m", "not integration"])


def cmd_integration_test(_args: argparse.Namespace) -> int:
    """`ci.toml [suites].integration` on the fast lane: only invoked by a
    workflow step gated on `contains(fromJSON(needs.precheck.outputs.
    fast_suites_json), 'integration')` (`[ci-test-integration]`/
    `[ci-full]`). Reuses the same `uv sync` editable install
    `cmd_python_test` already performed earlier in the same job — the
    fast lane never installs twice. See `tests/integration/README.md`."""
    return _run(
        ["uv", "run", "--no-sync", "pytest", "-m", "integration", "tests/integration"]
    )


def cmd_wheel_build(args: argparse.Namespace) -> int:
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    rc = _run(["soldr", "wheel", "--out", str(out_dir)])
    if rc != 0:
        return rc
    wheels = sorted(out_dir.glob("*.whl"))
    if not wheels:
        print(f"ci/fast.py: soldr wheel produced no .whl in {out_dir}", file=sys.stderr)
        return 1
    wheel_path = wheels[-1]
    print(f"wheel: {wheel_path}")
    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            fh.write(f"wheel-path={wheel_path}\n")
    return 0


def cmd_wheel_install(args: argparse.Namespace) -> int:
    """Clean-venv install of the built wheel — proves the packaged
    artifact, not the source tree, imports and runs (`PKG-001`)."""
    wheels = sorted(Path(args.wheel_dir).glob("*.whl"))
    if not wheels:
        print(f"ci/fast.py: no wheel found in {args.wheel_dir}", file=sys.stderr)
        return 1
    wheel = wheels[-1]
    venv_dir = Path(args.venv)
    rc = _run(["uv", "venv", str(venv_dir), "--python", args.python])
    if rc != 0:
        return rc
    # Linux-only lane (ubuntu-24.04): no sys.platform/os.name branch needed
    # here — see ci.toml [allow] platform-code / CLAUDE.md rule 8.
    python_bin = venv_dir / "bin" / "python3"
    return _run(["uv", "pip", "install", "--python", str(python_bin), str(wheel)])


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("build-json")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_build_json)

    p = sub.add_parser("rust-test")
    p.set_defaults(func=cmd_rust_test)

    p = sub.add_parser("python-sync")
    p.set_defaults(func=cmd_python_sync)

    p = sub.add_parser("python-test")
    p.set_defaults(func=cmd_python_test)

    p = sub.add_parser("integration-test")
    p.set_defaults(func=cmd_integration_test)

    p = sub.add_parser("wheel-build")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_wheel_build)

    p = sub.add_parser("wheel-install")
    p.add_argument("--wheel-dir", required=True)
    p.add_argument("--venv", required=True)
    p.add_argument("--python", default="3.11")
    p.set_defaults(func=cmd_wheel_install)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
