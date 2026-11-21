"""Local gate entry point: `python ci/local_gate.py --lane <id>`.

`local-gate.toml` declares every lane as this one command plus
`--lane <id>` (GATE-007: the lanes together are the gate). Two kinds of
lane exist:

* `precheck` and `fast` replay their `.github/workflows/ci.yml` job through
  Bosn -> act2 (`bosn ci run ... --job <id> --json`). The process is
  replaced by Bosn, so its `--json` report reaches stdout untouched for the
  `[gate.replay]` evidence check.
* `lint` runs `soldr lint` on the host. Lint never runs under Bosn: a
  container replay's caching semantics break this machine, so formatting,
  Clippy (host plus every `[workspace.metadata.soldr].targets` triple) and
  every Dylint library (`[workspace.metadata.dylint]`, also per declared
  cross target via `--cross-dylint`) run through the host's `soldr lint`.
  That covers what the remote `ci.yml:dylint` job (ci/dylint.py) checks.

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


def lane_command(lane: str) -> list[str]:
    """The argv one lane runs."""
    if lane == LINT_LANE:
        return ["soldr", "lint", "rust", "--check", "--cross-dylint"]
    if lane in BOSN_LANES:
        return [
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
    raise ValueError(f"unknown lane {lane!r}; expected one of {', '.join(LANES)}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lane", choices=LANES, default=None)
    args = parser.parse_args(argv)

    if args.lane is not None:
        command = lane_command(args.lane)
        print(f"[local_gate] {args.lane}: {' '.join(command)}", file=sys.stderr)
        sys.stderr.flush()
        try:
            os.execvp(command[0], command)
        except OSError as error:
            print(f"[local_gate] cannot start {command[0]}: {error}", file=sys.stderr)
            return 1

    for lane in LANES:
        command = lane_command(lane)
        print(f"[local_gate] {lane}: {' '.join(command)}", file=sys.stderr)
        try:
            returncode = subprocess.run(command, check=False).returncode
        except OSError as error:
            print(f"[local_gate] cannot start {command[0]}: {error}", file=sys.stderr)
            return 1
        if returncode != 0:
            return returncode
    return 0


if __name__ == "__main__":
    sys.exit(main())
