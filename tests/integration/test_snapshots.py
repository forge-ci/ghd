"""#13: planning snapshots share repository data through the installed CLI."""

from __future__ import annotations

import json
import subprocess
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

ResourceCli = tuple[Callable[..., subprocess.CompletedProcess[str]], Path]


def test_snapshot_repository_reads_share_one_cold_request(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    for _ in range(2):
        response = invoke("snapshot", "-R", "o/r", native=True)
        assert response.returncode == 0, response.stderr
        document = json.loads(response.stdout)
        assert document["repository"]["nameWithOwner"] == "o/r"
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 1


def test_concurrent_snapshots_share_one_request(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    with ThreadPoolExecutor(max_workers=16) as pool:
        replies = list(
            pool.map(lambda _: invoke("snapshot", "-R", "o/r", native=True), range(16))
        )
    assert all(reply.returncode == 0 for reply in replies), [
        reply.stderr for reply in replies if reply.returncode
    ]
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 1
    sources = [json.loads(reply.stdout)["freshness"]["source"] for reply in replies]
    assert sources.count("fetched") == 1
    assert set(sources) <= {"fetched", "fresh", "shared"}


def test_fresh_bypasses_the_shared_window_and_fails_without_cached_output(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("snapshot", "-R", "o/r")
    first = invoke(*args, native=True)
    assert first.returncode == 0, first.stderr
    trace.with_name("snapshot-title").write_text("new title", encoding="utf-8")
    cached = invoke(*args, native=True)
    assert (
        json.loads(cached.stdout)["repository"]["pullRequests"]["nodes"][0]["title"]
        == "snapshot title"
    )
    fresh = invoke(*args, "--fresh", native=True)
    assert fresh.returncode == 0, fresh.stderr
    assert (
        json.loads(fresh.stdout)["repository"]["pullRequests"]["nodes"][0]["title"]
        == "new title"
    )
    trace.with_name("snapshot-fail").touch()
    failed = invoke(*args, "--fresh", native=True)
    assert failed.returncode == 3
    assert not failed.stdout
    calls = [
        json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()
    ]
    assert len(calls) == 3
    assert "Cache-Control: no-cache" in calls[-1] and "Pragma: no-cache" in calls[-1]


@pytest.mark.parametrize("resource_cli", [{"GHD_FRESH_MS": "0"}], indirect=True)
def test_expired_planning_data_requires_opt_in(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    args = ("snapshot", "-R", "o/r")
    first = invoke(*args, native=True)
    assert first.returncode == 0, first.stderr
    trace.with_name("snapshot-fail").touch()
    refused = invoke(*args, native=True)
    assert refused.returncode == 3 and not refused.stdout
    accepted = invoke(*args, "--allow-stale", native=True)
    assert accepted.returncode == 0, accepted.stderr
    document = json.loads(accepted.stdout)
    assert document["freshness"]["stale"] is True
    assert document["freshness"]["source"] == "stale"
    assert document["freshness"]["age_ms"] >= 0
    assert "unavailable" in document["freshness"]["reason"]


@pytest.mark.parametrize(
    "marker",
    [
        "snapshot-empty",
        "snapshot-truncated",
        "snapshot-root-truncated",
        "snapshot-no-checks",
    ],
)
def test_empty_repositories_and_truncated_connections_are_explicit(
    resource_cli: ResourceCli, marker: str
) -> None:
    invoke, trace = resource_cli
    trace.with_name(marker).touch()
    response = invoke("snapshot", "-R", "o/r", native=True)
    assert response.returncode == 0, response.stderr
    document = json.loads(response.stdout)
    repo = document["repository"]
    if marker == "snapshot-empty":
        assert repo["defaultBranchRef"] is None
        assert repo["pullRequests"]["nodes"] == []
        assert document["truncated"] is False
    elif marker == "snapshot-no-checks":
        assert (
            repo["pullRequests"]["nodes"][0]["commits"]["nodes"][0]["commit"][
                "statusCheckRollup"
            ]
            is None
        )
    else:
        assert document["truncated"] is True
        if marker == "snapshot-truncated":
            assert (
                repo["pullRequests"]["nodes"][0]["reviews"]["pageInfo"][
                    "hasPreviousPage"
                ]
                is True
            )
        else:
            assert repo["pullRequests"]["pageInfo"]["hasNextPage"] is True


@pytest.mark.parametrize(
    "marker",
    ["snapshot-missing-pages", "snapshot-head-mismatch", "snapshot-partial-error"],
)
def test_incomplete_or_inconsistent_authority_response_is_refused(
    resource_cli: ResourceCli, marker: str
) -> None:
    invoke, trace = resource_cli
    trace.with_name(marker).touch()
    response = invoke("snapshot", "-R", "o/r", "--fresh", native=True)
    assert response.returncode == 3
    assert not response.stdout
    assert response.stderr


@pytest.mark.parametrize(
    "marker", ["snapshot-stale", "snapshot-stale-no-age", "snapshot-age"]
)
def test_downstream_stale_responses_cannot_become_authority(
    resource_cli: ResourceCli, marker: str
) -> None:
    invoke, trace = resource_cli
    trace.with_name(marker).write_text("1", encoding="utf-8")
    fresh = invoke("snapshot", "-R", "o/r", "--fresh", native=True)
    assert fresh.returncode == 3 and not fresh.stdout
    planning = invoke("snapshot", "-R", "o/r", "--allow-stale", native=True)
    assert planning.returncode == 0, planning.stderr
    metadata = json.loads(planning.stdout)["freshness"]
    assert metadata["stale"] is True
    if marker == "snapshot-stale-no-age":
        assert metadata["age_ms"] is None
    else:
        assert metadata["age_ms"] >= 1000


@pytest.mark.parametrize("directive", ["no-store", "no-cache", "max-age=0"])
def test_snapshot_origin_policy_limits_reuse(
    resource_cli: ResourceCli, directive: str
) -> None:
    invoke, trace = resource_cli
    trace.with_name("snapshot-cache-control").write_text(directive, encoding="utf-8")
    for _ in range(2):
        response = invoke("snapshot", "-R", "o/r", native=True)
        assert response.returncode == 0, response.stderr
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 2


def test_binding_and_repo_case_alias_share_snapshot_data(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    cli = invoke("snapshot", "-R", "O/R", native=True)
    assert cli.returncode == 0, cli.stderr
    code = (
        "from ghd.bindings import snapshot; import json; r=snapshot('o/r'); "
        "print(json.dumps({'repository':r.data['repository'],"
        "'exit_code':r.exit_code,'source':r.source}))"
    )
    binding = invoke(python_code=code)
    assert binding.returncode == 0, binding.stderr
    result = json.loads(binding.stdout)
    assert result["exit_code"] == 0
    assert result["repository"] == json.loads(cli.stdout)["repository"]
    assert result["source"] == "fresh"
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 1


def test_mutation_invalidates_snapshot_even_for_stale_planning(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("snapshot", "-R", "o/r")
    first = invoke(*args, native=True)
    assert first.returncode == 0, first.stderr
    mutation = invoke("issue", "edit", "42", "-R", "o/r", "--title", "changed")
    assert mutation.returncode == 0, mutation.stderr
    trace.with_name("snapshot-fail").touch()
    response = invoke(*args, "--allow-stale", native=True)
    assert response.returncode == 3, response.stderr
    assert not response.stdout
