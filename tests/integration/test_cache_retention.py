"""#11: retained resources obey shared entry/byte capacity, including zero."""

from __future__ import annotations

import json
import subprocess
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

ResourceCli = tuple[Callable[..., subprocess.CompletedProcess[str]], Path]


def cold_count(trace: Path) -> int:
    return len(trace.read_text(encoding="utf-8").splitlines())


@pytest.mark.parametrize(
    "resource_cli", [{"GHD_CACHE_MAX_ENTRIES": "1"}], indirect=True
)
def test_entry_capacity_evicts_the_least_recent_resource(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    for number in (42, 43, 42):
        response = invoke(
            "issue", "view", str(number), "-R", "o/r", "--json", "number,title"
        )
        assert response.returncode == 0, response.stderr
        assert json.loads(response.stdout)["number"] == number
    assert cold_count(trace) == 3, "one retained entry cannot keep both resources"


@pytest.mark.parametrize(
    "resource_cli",
    [
        {"GHD_CACHE_MAX_BYTES": "0"},
        {"GHD_CACHE_MAX_ENTRIES": "0"},
        {"GHD_CACHE_MAX_BYTES": "1100"},
    ],
    indirect=True,
)
def test_zero_byte_capacity_disables_retention(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    for _ in range(2):
        response = invoke("issue", "view", "42", "-R", "o/r", "--json", "title")
        assert response.returncode == 0, response.stderr
    assert cold_count(trace) == 2, "disabled capacity must not return retained bytes"


@pytest.mark.parametrize(
    "resource_cli", [{"GHD_CACHE_MAX_ENTRIES": "1"}], indirect=True
)
def test_rest_and_resources_share_capacity(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    for _ in range(2):
        for args in (
            ("issue", "view", "42", "-R", "o/r", "--json", "title"),
            ("api", "fixture/body"),
        ):
            response = invoke(*args)
            assert response.returncode == 0, response.stderr
    assert cold_count(trace) == 4, (
        "separate cache types cannot each claim the same budget"
    )
    stats = invoke("stats", native=True)
    assert stats.returncode == 0, stats.stderr
    assert json.loads(stats.stdout)["cache_entries"] == 1


@pytest.mark.parametrize("resource_cli", [{"GHD_CACHE_DISK": "1"}], indirect=True)
def test_restart_revalidates_retained_bytes_before_reuse(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    first = invoke("api", "fixture/body")
    assert first.returncode == 0, first.stderr
    restarted = invoke("api", "fixture/body", daemon_action="restart")
    assert restarted.returncode == 0, restarted.stderr
    assert restarted.stdout == first.stdout
    calls = [
        json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()
    ]
    assert len(calls) == 2, (
        "restart data needs an origin validation, even within the TTL"
    )
    assert any(word.startswith("If-None-Match:") for word in calls[1]), calls
    again = invoke("api", "fixture/body")
    assert again.returncode == 0, again.stderr
    assert cold_count(trace) == 2, (
        "validated restart body can enter the short fresh window"
    )


@pytest.mark.parametrize(
    "resource_cli", [{"GHD_CACHE_MAX_ENTRIES": "2"}], indirect=True
)
def test_concurrent_reads_and_evictions_bound_slot_bookkeeping(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli

    def read(number: int) -> subprocess.CompletedProcess[str]:
        return invoke("issue", "view", str(number), "-R", "o/r", "--json", "title")

    with ThreadPoolExecutor(max_workers=16) as pool:
        responses = list(pool.map(read, [42] * 16))
        assert all(response.returncode == 0 for response in responses), [
            response.stderr for response in responses if response.returncode
        ]
        assert cold_count(trace) == 1, "identical admitted requests still coalesce"
    # Three distinct concurrent keys still race a two-entry index, without
    # exceeding the governor's advertised three routine dispatch slots.
    with ThreadPoolExecutor(max_workers=3) as pool:
        responses = list(pool.map(read, range(43, 75)))
        assert all(response.returncode == 0 for response in responses), [
            response.stderr for response in responses if response.returncode
        ]
    stats = invoke("stats", native=True)
    assert stats.returncode == 0, stats.stderr
    assert json.loads(stats.stdout)["cache_entries"] <= 2
    assert json.loads(stats.stdout)["cache_charged_bytes"] < 67108864


@pytest.mark.parametrize("resource_cli", [{"GHD_CACHE_DISK": "1"}], indirect=True)
def test_cli_mutation_with_no_daemon_invalidates_restart_validator(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    first = invoke("api", "fixture/body")
    assert first.returncode == 0, first.stderr
    mutation = invoke(
        "issue", "edit", "42", "-R", "o/r", "--title", "changed", daemon_action="stop"
    )
    assert mutation.returncode == 0, mutation.stderr
    restarted = invoke("api", "fixture/body", daemon_action="restart")
    assert restarted.returncode == 0, restarted.stderr
    calls = [
        json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()
    ]
    api_calls = [args for args in calls if args[:2] == ["api", "fixture/body"]]
    assert len(api_calls) == 2
    assert not any(word.startswith("If-None-Match:") for word in api_calls[1]), (
        api_calls
    )


@pytest.mark.parametrize("resource_cli", [{"GHD_CACHE_DISK": "1"}], indirect=True)
@pytest.mark.parametrize("directive", ["no-store", "no-cache", "max-age=0"])
def test_collection_origin_policy_survives_restart(
    resource_cli: ResourceCli, directive: str
) -> None:
    invoke, trace = resource_cli
    trace.with_name("collection-policy").write_text(directive, encoding="utf-8")
    args = ("run", "list", "-R", "o/r", "--json", "databaseId,attempt", "--limit", "1")
    for restart in (False, False, True):
        response = invoke(*args, daemon_action="restart" if restart else None)
        assert response.returncode == 0, response.stderr
        assert json.loads(response.stdout) == [{"attempt": 1, "databaseId": 42}]
    calls = [
        json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()
    ]
    assert len(calls) == 3, "origin policy forbids an unvalidated fresh hit"
    if directive == "no-store":
        assert not any(
            word.startswith("If-None-Match:") for args in calls for word in args
        ), calls
    else:
        assert any(word.startswith("If-None-Match:") for word in calls[-1]), calls


@pytest.mark.parametrize("resource_cli", [{"GHD_CACHE_DISK": "1"}], indirect=True)
def test_failed_restart_validation_never_serves_a_retained_body(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    initial = invoke("api", "fixture/body")
    assert initial.returncode == 0, initial.stderr
    trace.with_name("revalidation-fails").touch()
    for restart in (True, False):
        failed = invoke(
            "api", "fixture/body", daemon_action="restart" if restart else None
        )
        assert failed.returncode != 0
        assert initial.stdout not in failed.stdout
        assert "unavailable" in failed.stderr
    stats = invoke("stats", native=True)
    assert stats.returncode == 0, stats.stderr
    assert json.loads(stats.stdout)["cache_charged_bytes"] < 2048


@pytest.mark.parametrize(
    "resource_cli", [{"GHD_CACHE_TTLS": '{"issue view":0}'}], indirect=True
)
def test_per_command_zero_ttl_does_not_disable_other_fresh_reads(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    for _ in range(2):
        for kind in ("issue", "pr"):
            response = invoke(kind, "view", "42", "-R", "o/r", "--json", "title")
            assert response.returncode == 0, response.stderr
    calls = [
        json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()
    ]
    assert sum(args[0] == "issue" for args in calls) == 2
    assert sum(args[0] == "pr" for args in calls) == 1


@pytest.mark.parametrize(
    "resource_cli",
    [{"GHD_FRESH_MS": "0", "GHD_CACHE_SETTLED_MS": "60000"}],
    indirect=True,
)
def test_completed_attempt_projection_does_not_freeze_mutable_metadata(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    stable = (
        "run",
        "view",
        "42",
        "-R",
        "o/r",
        "--attempt",
        "1",
        "--json",
        "headSha,status",
    )
    for _ in range(2):
        response = invoke(*stable)
        assert response.returncode == 0, response.stderr
    assert cold_count(trace) == 1
    for _ in range(2):
        response = invoke(
            "run",
            "view",
            "42",
            "-R",
            "o/r",
            "--attempt",
            "1",
            "--json",
            "workflowName,status",
        )
        assert response.returncode == 0, response.stderr
    assert cold_count(trace) == 3, (
        "mutable metadata always follows the ordinary short policy"
    )
    other = invoke(
        "run", "view", "42", "-R", "o/r", "--attempt", "2", "--json", "headSha,status"
    )
    assert other.returncode == 0, other.stderr
    assert cold_count(trace) == 4, "completed attempts remain distinct"
