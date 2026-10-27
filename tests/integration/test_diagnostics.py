"""#15: bounded redacted diagnostics and network-free replay."""

from __future__ import annotations

import http.client
import json
import subprocess
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

ResourceCli = tuple[Callable[..., subprocess.CompletedProcess[str]], Path]


def test_request_ledger_tracks_safe_outcomes_and_dashboard(
    resource_cli: ResourceCli,
) -> None:
    invoke, _ = resource_cli
    args = ("issue", "view", "42", "-R", "o/r", "--json", "title")
    for _ in range(2):
        assert invoke(*args).returncode == 0
    reply = invoke("ledger", native=True)
    assert reply.returncode == 0, reply.stderr
    report = json.loads(reply.stdout)
    assert report["schema"] == 1 and report["capacity"] == 256
    rows = report["entries"]
    assert [row["source"] for row in rows] == ["miss", "hit"]
    assert all(row["command"] == "issue view" for row in rows)
    assert all(row["response_bytes"] > 0 and row["latency_us"] >= 0 for row in rows)
    assert "resource-fixture-token" not in reply.stdout
    assert "title-42" not in reply.stdout
    page = invoke("dashboard", native=True)
    assert page.returncode == 0 and "<!doctype html>" in page.stdout
    assert "resource-fixture-token" not in page.stdout


def test_replay_is_offline_and_labels_exact_key_modeling(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("issue", "view", "42", "-R", "o/r", "--json", "title")
    assert invoke(*args).returncode == 0
    assert invoke(*args).returncode == 0
    report = invoke("ledger", native=True)
    workload = trace.with_name("workload.json")
    workload.write_text(report.stdout, encoding="utf-8")
    before = trace.read_bytes()
    replay = invoke(
        "replay", str(workload), "--ttl", "60000", native=True, daemon_action="stop"
    )
    assert replay.returncode == 0, replay.stderr
    result = json.loads(replay.stdout)
    assert result["network_free"] is True
    assert result["modeled_exact_key_hits"] == 1
    assert result["provider_points_saved"] is None
    assert len(result["input_blake3"]) == 64
    assert trace.read_bytes() == before


@pytest.mark.parametrize("resource_cli", [{"GHD_LOG_ARGS": "1"}], indirect=True)
def test_opt_in_arguments_remain_redacted_and_ledger_is_bounded(
    resource_cli: ResourceCli,
) -> None:
    invoke, _ = resource_cli
    code = (
        "from ghd.bindings import gh_read; "
        "args=['api','fixture/body','-H','X-Private: sensitive-header']; "
        "[(gh_read(args)) for _ in range(270)]"
    )
    response = invoke(python_code=code)
    assert response.returncode == 0, response.stderr
    report = invoke("ledger", native=True)
    assert report.returncode == 0, report.stderr
    document = json.loads(report.stdout)
    assert len(document["entries"]) == 256 and document["dropped"] == 14
    assert all(row["args"] is not None for row in document["entries"])
    assert "sensitive-header" not in report.stdout and "X-Private" not in report.stdout


def test_concurrent_reads_report_coalescing_and_callers(
    resource_cli: ResourceCli,
) -> None:
    invoke, _ = resource_cli
    args = ("snapshot", "-R", "o/r")
    with ThreadPoolExecutor(max_workers=8) as pool:
        responses = list(
            pool.map(
                lambda n: invoke(*args, native=True, context={"GHD_CALLER_ID": str(n)}),
                range(8),
            )
        )
    assert all(response.returncode == 0 for response in responses)
    ledger = invoke("ledger", native=True)
    rows = json.loads(ledger.stdout)["entries"]
    assert len({row["caller"] for row in rows}) == 8
    assert sum(row["source"] == "miss" for row in rows) == 1
    assert any(row["source"] == "coalesced" for row in rows)


def test_dashboard_auth_and_cross_origin_writes(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    endpoint = next((trace.parent / "state").glob("identities-v1/*/endpoint"))
    port, token = endpoint.read_text(encoding="utf-8").split()
    connection = http.client.HTTPConnection("127.0.0.1", int(port), timeout=5)
    try:
        connection.request("GET", "/v1/dashboard")
        response = connection.getresponse()
        assert response.status == 401
        response.read()
        connection.request(
            "POST",
            "/v1/admin/stop",
            headers={
                "Authorization": f"Bearer {token}",
                "X-Ghd-Identity": endpoint.parent.name,
                "Origin": "https://foreign.invalid",
            },
        )
        response = connection.getresponse()
        assert response.status == 403
        response.read()
    finally:
        connection.close()
    assert invoke("daemon", "status", native=True).returncode == 0


@pytest.mark.parametrize("resource_cli", [{"GHD_FRESH_MS": "0"}], indirect=True)
def test_ledger_distinguishes_conditional_304(resource_cli: ResourceCli) -> None:
    invoke, _ = resource_cli
    for _ in range(2):
        response = invoke("api", "fixture/body")
        assert response.returncode == 0, response.stderr
    report = invoke("ledger", native=True)
    assert [row["source"] for row in json.loads(report.stdout)["entries"]] == [
        "miss",
        "304",
    ]


@pytest.mark.parametrize(
    "resource_cli", [{"GHD_CACHE_MAX_ENTRIES": "1"}], indirect=True
)
def test_evictions_and_refused_requests_are_visible(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    for number in (42, 43):
        assert (
            invoke(
                "issue", "view", str(number), "-R", "o/r", "--json", "title"
            ).returncode
            == 0
        )
    trace.with_name("snapshot-fail").touch()
    refused = invoke("snapshot", "-R", "o/r", "--fresh", native=True)
    assert refused.returncode == 3
    reply = invoke("ledger", native=True)
    report = json.loads(reply.stdout)
    assert report["evictions"] >= 1
    assert report["entries"][-1]["source"] == "refused"


def test_repeated_mutations_never_become_modeled_cache_hits(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    workload = trace.with_name("mutations.json")
    rows = [
        {
            "at_ms": n,
            "key": "a" * 64,
            "source": "miss",
            "operation_kind": "write",
            "cache_eligible": False,
            "upstream_dispatches": 1,
        }
        for n in range(2)
    ]
    workload.write_text(json.dumps({"schema": 1, "entries": rows}), encoding="utf-8")
    response = invoke("replay", str(workload), "--ttl", "60000", native=True)
    assert response.returncode == 0, response.stderr
    result = json.loads(response.stdout)
    assert result["modeled_exact_key_hits"] == 0
    assert result["eligible_requests"] == 0
    assert result["observed_known_dispatches"] == 2


def test_authority_reads_are_excluded_from_replay_reuse(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    for _ in range(2):
        response = invoke("snapshot", "-R", "o/r", "--fresh", native=True)
        assert response.returncode == 0, response.stderr
    ledger = invoke("ledger", native=True)
    workload = trace.with_name("authority.json")
    workload.write_text(ledger.stdout, encoding="utf-8")
    replay = invoke("replay", str(workload), "--ttl", "60000", native=True)
    assert replay.returncode == 0, replay.stderr
    result = json.loads(replay.stdout)
    assert result["modeled_exact_key_hits"] == 0
    assert result["eligible_requests"] == 0
