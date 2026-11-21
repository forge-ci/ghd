"""Lane-shape contract for `local-gate.toml` and `ci/local_gate.py`.

Lint never runs under Bosn: the `lint` lane runs host `soldr lint`, no lane
replays a lint job, and gate trust credits the remote `dylint` job to that
lane.
"""

from __future__ import annotations

import importlib
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT))

# tomllib is stdlib from 3.11; requires-python still admits 3.10.
tomllib = pytest.importorskip("tomllib")
local_gate = importlib.import_module("ci.local_gate")
act_orchestrate = importlib.import_module("ci.localrun.act_orchestrate")


def _gate() -> dict:
    with (REPO_ROOT / "local-gate.toml").open("rb") as fh:
        return tomllib.load(fh)["gate"]


def test_declared_lanes_match_script() -> None:
    gate = _gate()
    assert tuple(gate["lanes"]) == local_gate.LANES
    for lane_id, lane in gate["lanes"].items():
        assert lane["run"] == [*gate["run"], "--lane", lane_id]


def test_lint_lane_runs_soldr_lint_on_host() -> None:
    assert local_gate.lane_command("lint") == [
        "soldr",
        "lint",
        "rust",
        "--check",
        "--cross-dylint",
    ]
    tools = _gate()["lanes"]["lint"]["tools"]
    assert "soldr" in tools
    assert "bosn" not in tools


@pytest.mark.parametrize("lane", local_gate.BOSN_LANES)
def test_bosn_lanes_never_select_a_lint_job(lane: str) -> None:
    command = local_gate.lane_command(lane)
    assert command[:3] == ["bosn", "ci", "run"]
    job = command[command.index("--job") + 1]
    assert job == lane
    assert job not in act_orchestrate.LINT_JOBS


def test_no_whole_workflow_full_run() -> None:
    assert "full-run" not in _gate()


def test_trust_credits_dylint_to_lint_lane() -> None:
    trust = _gate()["trust"]
    assert trust["covered-by"]["ci.yml:dylint"] == ["lint"]


def test_act_refuses_lint_jobs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    passed = SimpleNamespace(exit_code=0)
    monkeypatch.setattr(act_orchestrate, "run_precheck", lambda *_a, **_k: passed)
    assert act_orchestrate.run_act(tmp_path, lanes_arg="dylint", title="") == 1
    assert act_orchestrate.run_act(tmp_path, lanes_arg=None, title="[ci-full] x") == 1
