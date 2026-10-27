"""#12: observed shared provider budget prevents another upstream dispatch."""

from __future__ import annotations

import json
import subprocess
from collections.abc import Callable
from pathlib import Path

import pytest

ResourceCli = tuple[Callable[..., subprocess.CompletedProcess[str]], Path]


@pytest.mark.parametrize(
    "resource_cli", [{"GHD_FRESH_MS": "0", "GHD_RATE_RESERVE": "100"}], indirect=True
)
def test_observed_budget_floor_refuses_the_next_uncached_read(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    trace.with_name("quota-remaining").write_text("99", encoding="utf-8")
    first = invoke("api", "fixture/body")
    assert first.returncode == 0, first.stderr
    refused = invoke("api", "fixture/body")
    assert refused.returncode != 0, "routine reads cannot spend the configured reserve"
    assert "budget" in refused.stderr.lower()
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 1


@pytest.mark.parametrize("resource_cli", [{"GHD_FRESH_MS": "0"}], indirect=True)
def test_cooldown_survives_daemon_restart_without_disk_body_cache(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    trace.with_name("provider-status").write_text("429", encoding="utf-8")
    trace.with_name("retry-after").write_text("120", encoding="utf-8")
    first = invoke("api", "fixture/body")
    assert first.returncode != 0
    second = invoke("api", "fixture/body", daemon_action="restart")
    assert second.returncode != 0
    assert "cooldown" in second.stderr.lower(), second.stderr
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 1


@pytest.mark.parametrize("resource_cli", [{"GHD_FRESH_MS": "0"}], indirect=True)
@pytest.mark.parametrize("remaining", ["5000", "0"])
def test_permission_denial_is_distinct_from_rate_limited_403(
    resource_cli: ResourceCli, remaining: str
) -> None:
    invoke, trace = resource_cli
    trace.with_name("provider-status").write_text("403", encoding="utf-8")
    trace.with_name("quota-remaining").write_text(remaining, encoding="utf-8")
    first = invoke("api", "fixture/body")
    second = invoke("api", "fixture/body")
    assert first.returncode != 0 and second.returncode != 0
    calls = len(trace.read_text(encoding="utf-8").splitlines())
    assert calls == (1 if remaining == "0" else 2)
    assert ("cooldown" in second.stderr.lower()) == (remaining == "0")


def test_caller_telemetry_preserves_shared_cached_bodies(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("issue", "view", "42", "-R", "o/r", "--json", "title")
    for caller in ("caller-a", "caller-b"):
        response = invoke(*args, context={"GHD_CALLER_ID": caller})
        assert response.returncode == 0, response.stderr
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 1
    stats = invoke("stats", native=True)
    assert stats.returncode == 0, stats.stderr
    governor = json.loads(stats.stdout)["governor"]
    assert len(governor["callers"]) == 2
    assert governor["totals"]["requests"] == 2
    assert governor["totals"]["cache_hits"] == 1
    assert governor["totals"]["gh_dispatches"] == 1
    assert "caller-a" not in stats.stdout and "caller-b" not in stats.stdout


@pytest.mark.parametrize(
    "resource_cli", [{"GHD_FRESH_MS": "0", "GHD_RATE_RESERVE": "100"}], indirect=True
)
def test_explicit_authority_read_can_use_positive_protected_reserve(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    trace.with_name("quota-remaining").write_text("99", encoding="utf-8")
    first = invoke("api", "fixture/body")
    assert first.returncode == 0, first.stderr
    refused = invoke("api", "fixture/body")
    assert refused.returncode != 0
    essential = invoke("api", "fixture/body", "--header", "Cache-Control: no-cache")
    assert essential.returncode == 0, essential.stderr
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 2


@pytest.mark.parametrize("resource_cli", [{"GHD_RATE_RESERVE": "100"}], indirect=True)
def test_fresh_cache_hit_needs_no_budget_but_forced_read_never_falls_back(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    trace.with_name("quota-remaining").write_text("0", encoding="utf-8")
    first = invoke("api", "fixture/body")
    assert first.returncode == 0, first.stderr
    cached = invoke("api", "fixture/body")
    assert cached.returncode == 0 and cached.stdout == first.stdout
    forced = invoke("api", "fixture/body", "--header", "Cache-Control: no-cache")
    assert forced.returncode != 0
    assert forced.stdout != first.stdout, (
        "authority refusal cannot contain a hidden cached answer"
    )
    assert "budget" in forced.stderr.lower()
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 1


@pytest.mark.parametrize("resource_cli", [{"GHD_FRESH_MS": "0"}], indirect=True)
def test_revalidation_telemetry_does_not_charge_304_as_provider_points(
    resource_cli: ResourceCli,
) -> None:
    invoke, _trace = resource_cli
    for _ in range(2):
        response = invoke("api", "fixture/body")
        assert response.returncode == 0, response.stderr
    stats = invoke("stats", native=True)
    assert stats.returncode == 0, stats.stderr
    governor = json.loads(stats.stdout)["governor"]
    assert governor["totals"]["requests"] == 2
    assert governor["totals"]["gh_dispatches"] == 2
    assert governor["totals"]["revalidated"] == 1
    assert governor["budgets"]["core"]["remaining_estimate"] == 5000
