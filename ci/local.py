#!/usr/bin/env python3
"""`ci/local.py` -- the one entry point for the local CI loop.

zackees/ci.yml#6 section 11 ("bosn -> act: local CI is first class").

    python3 ci/local.py precheck              # ~seconds; the agent Stop-hook / pre-push gate
    python3 ci/local.py act                   # precheck, then the complete PR workflow via Bosn -> act2
    python3 ci/local.py act --lanes fast      # selected-job diagnostic
    python3 ci/local.py act --title "[ci-full] ..."  # exercise a different tag selection

All logic lives in `ci/localrun/` (stdlib only, except where it shells
out to `uv run --no-project --with pyyaml` for `ci_lint` itself, which
needs PyYAML) -- this file is only argument parsing and dispatch, same
convention as `ci.py`/`ci/gates/`.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

# pylint: disable-next=wrong-import-position
from ci.localrun.act_orchestrate import run_act  # noqa: E402

# pylint: disable-next=wrong-import-position
from ci.localrun.precheck import run_precheck  # noqa: E402


def _cmd_precheck(args: argparse.Namespace) -> int:
    return run_precheck(REPO_ROOT, title=args.title or "").exit_code


def _cmd_act(args: argparse.Namespace) -> int:
    return run_act(REPO_ROOT, lanes_arg=args.lanes, title=args.title or "")


def _cmd_act_inner(_args: argparse.Namespace) -> int:
    print(
        "act-inner is retired; use python3 ci/local.py act for Bosn → act2",
        file=sys.stderr,
    )
    return 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="ci/local.py")
    sub = parser.add_subparsers(dest="command", required=True)

    p_precheck = sub.add_parser(
        "precheck", help="ci_lint precheck --local (agent gate)"
    )
    p_precheck.add_argument(
        "--title", default=None, help="PR title to check tags against"
    )
    p_precheck.set_defaults(func=_cmd_precheck)

    p_act = sub.add_parser(
        "act", help="precheck, then the complete PR workflow through Bosn -> act2"
    )
    p_act.add_argument(
        "--lanes",
        default=None,
        help="comma-separated job IDs for diagnostics (default: complete PR workflow)",
    )
    p_act.add_argument(
        "--title", default=None, help="PR title driving the act event (tags -> plan)"
    )
    p_act.set_defaults(func=_cmd_act)

    p_inner = sub.add_parser(
        "act-inner", help=argparse.SUPPRESS
    )  # container-only, see ci/localrun/act_inner.py
    p_inner.set_defaults(func=_cmd_act_inner)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
