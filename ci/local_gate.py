"""Local gate entry point: `python ci/local_gate.py --lane <id>`.

`local-gate.toml` declares every lane as this one command plus
`--lane <id>` (GATE-007: the lanes together are the gate). Two kinds of
lane exist:

* `precheck` and `fast` replay their `.github/workflows/ci.yml` job through
  Bosn -> act2 (`bosn ci run ... --job <id> --json`). The process is
  replaced by Bosn, so its `--json` report reaches stdout untouched for the
  `[gate.replay]` evidence check.
* `lint` runs on the host. Lint never runs under Bosn: a container
  replay's caching semantics break this machine. `soldr lint` covers
  formatting, Clippy (host plus every `[workspace.metadata.soldr].targets`
  triple) and every Dylint library (`[workspace.metadata.dylint]`, also
  per declared cross target via `--cross-dylint`); ruff and pylint then run
  through uv exactly as `./ci.py ruff` / `./ci.py pylint` do, after
  `ci/fast.py python-sync` builds `_native` for pylint. Together that is
  every check the remote `ci.yml:dylint` job (the repository's lint job)
  runs, and the Bosn-replayed `fast` job runs none of them.

With no `--lane`, every lane runs in declaration order and the first failure
stops the run.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys

WORKFLOW = ".github/workflows/ci.yml"

BOSN_LANES = ("precheck", "fast")
LINT_LANE = "lint"
LANES = (*BOSN_LANES, LINT_LANE)


LINT_COMMANDS: tuple[tuple[str, ...], ...] = (
    ("soldr", "lint", "rust", "--check", "--cross-dylint"),
    ("python3", "ci/gates/ruff.py"),
    ("python3", "ci/fast.py", "python-sync"),
    ("python3", "ci/gates/pylint.py"),
)


def lane_commands(lane: str) -> list[list[str]]:
    """The argv list one lane runs, in order; the first failure stops it."""
    if lane == LINT_LANE:
        return [list(command) for command in LINT_COMMANDS]
    if lane in BOSN_LANES:
        return [
            [
                "bosn",
                "ci",
                "run",
                "--workspace",
                ".",
                "--workflow",
                WORKFLOW,
                "--trigger",
                "pr",
                "--wait",
                "--json",
                "--job",
                lane,
            ]
        ]
    raise ValueError(f"unknown lane {lane!r}; expected one of {', '.join(LANES)}")


def _run_lane(lane: str) -> int:
    for command in lane_commands(lane):
        print(f"[local_gate] {lane}: {' '.join(command)}", file=sys.stderr)
        sys.stderr.flush()
        try:
            returncode = subprocess.run(command, check=False).returncode
        except OSError as error:
            print(f"[local_gate] cannot start {command[0]}: {error}", file=sys.stderr)
            return 1
        if returncode != 0:
            return returncode
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lane", choices=LANES, default=None)
    args = parser.parse_args(argv)

    if args.lane in BOSN_LANES:
        # Replace this process so Bosn's `--json` report reaches stdout
        # untouched for the `[gate.replay]` evidence check.
        (command,) = lane_commands(args.lane)
        print(f"[local_gate] {args.lane}: {' '.join(command)}", file=sys.stderr)
        sys.stderr.flush()
        try:
            os.execvp(command[0], command)
        except OSError as error:
            print(f"[local_gate] cannot start {command[0]}: {error}", file=sys.stderr)
            return 1

    for lane in (args.lane,) if args.lane else LANES:
        returncode = _run_lane(lane)
        if returncode != 0:
            return returncode
    return 0


if __name__ == "__main__":
    sys.exit(main())
