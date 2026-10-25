#!/usr/bin/env -S uv run --no-project --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml>=6"]
# ///
"""Canonical CI gate dispatcher.

Usage:
    ./ci.py <gate>     # run one gate
    ./ci.py all        # run every gate, continue past failures
    ./ci.py --list     # show registered gates in order

Every gate is `ci/gates/<name>.py` exposing `def run() -> int`.
GATE_ORDER below is the canonical sequence; `all` runs them in that
order with continue-past-failure semantics, except that a failing
`build` gate halts the run (downstream gates against an uncompiled
tree only produce noise).

See zackees/zccache#835 rule 6 for the rationale: keep ci.yml thin,
push logic to Python, lock the contract so `./ci.py fmt` on a laptop
runs the exact same bytes as the GHA step.
"""

from __future__ import annotations

import argparse
import importlib
import sys
import traceback
from pathlib import Path

ROOT = Path(__file__).resolve().parent

# Order matters: cheap workspace-state checks first, then language-level
# linters, then the build, then tests, then artifact-shape gates.
# `build` is the only fatal gate — see the loop below.
GATE_ORDER: list[str] = [
    "loc",
    "fmt",
    "clippy",
    "ruff",
    "build",
    "pylint",
    "test",
    "backend_smoke",
    "action_yaml",
    "action_surface",
]

FATAL_GATES = {"build"}


def _load_gate(name: str):
    sys.path.insert(0, str(ROOT))
    return importlib.import_module(f"ci.gates.{name}")


def run_one(name: str) -> int:
    print(f"==> [{name}]", flush=True)
    try:
        mod = _load_gate(name)
    except ModuleNotFoundError as exc:
        print(f"[{name}] unknown gate: {exc}", file=sys.stderr)
        return 1
    try:
        rc = mod.run()
    except Exception:
        print(f"[{name}] crashed:", file=sys.stderr)
        traceback.print_exc()
        return 1
    if rc != 0:
        print(f"[{name}] FAILED (rc={rc})", file=sys.stderr)
    else:
        print(f"[{name}] ok", flush=True)
    return rc


def run_all() -> int:
    failures: list[str] = []
    for name in GATE_ORDER:
        rc = run_one(name)
        if rc != 0:
            failures.append(name)
            if name in FATAL_GATES:
                print(
                    f"\n[{name}] is fatal; halting (downstream gates would produce noise against an uncompiled tree).",
                    file=sys.stderr,
                )
                break
    print("")
    if failures:
        print(f"FAILED: {', '.join(failures)}", file=sys.stderr)
        return 1
    print("ALL GATES PASSED")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(prog="ci.py", description=__doc__)
    parser.add_argument(
        "gate",
        nargs="?",
        default="all",
        help="gate name (see --list) or 'all' (default)",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="print registered gates in run order and exit",
    )
    args = parser.parse_args()

    if args.list:
        for name in GATE_ORDER:
            print(name)
        return 0

    if args.gate == "all":
        return run_all()

    if args.gate not in GATE_ORDER:
        print(
            f"Unknown gate: {args.gate}. Run `./ci.py --list` for the registered gates.",
            file=sys.stderr,
        )
        return 2

    return run_one(args.gate)


if __name__ == "__main__":
    raise SystemExit(main())
