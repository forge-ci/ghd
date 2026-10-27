"""#10: safe REST body retention and exact request identity."""

from __future__ import annotations

import json
import shutil
import subprocess
from collections.abc import Callable
from pathlib import Path

import pytest

from ghd.platforms import is_windows

ResourceCli = tuple[Callable[..., subprocess.CompletedProcess[str]], Path]


@pytest.mark.skipif(
    is_windows() or shutil.which("gh") is None, reason="real gh local Unix transport"
)
@pytest.mark.parametrize("resource_cli", ["golden"], indirect=True)
def test_rest_get_caches_the_response_body(resource_cli: ResourceCli) -> None:
    """#10 RED: identical safe REST GETs currently execute twice."""
    invoke, trace = resource_cli
    first = invoke("api", "repos/o/r/actions/runs?per_page=1")
    second = invoke("api", "repos/o/r/actions/runs?per_page=1")
    assert first.returncode == second.returncode == 0, (first.stderr, second.stderr)
    assert first.stdout == second.stdout
    assert json.loads(first.stdout)["workflow_runs"][0]["id"] == 42
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 1, (
        "safe REST reads must share their retained body"
    )


ENDPOINT = "repos/o/r/actions/runs?per_page=1"


def calls(trace: Path) -> list[dict[str, object]]:
    return [json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()]


@pytest.mark.parametrize("resource_cli", ["golden-zero"], indirect=True)
def test_rest_304_retains_body_and_reports_network_separately_from_points(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    first = invoke("api", ENDPOINT, context={"GHD_DEBUG": "1"})
    second = invoke("api", ENDPOINT, context={"GHD_DEBUG": "1"})
    assert first.returncode == second.returncode == 0, (first.stderr, second.stderr)
    assert first.stdout == second.stdout
    assert first.stderr == "ghd: fetched\n"
    assert second.stderr == "ghd: revalidated\n"
    assert len(calls(trace)) == 2
    assert calls(trace)[1]["validator"] is not None
    stats = invoke("stats", native=True)
    assert stats.returncode == 0, stats.stderr
    data = json.loads(stats.stdout)
    assert data["rest_http_operations"] == data["rest_http_responses"] == 2
    assert data["rest_304"] == 1
    assert data["rest_provider_budgets"]["core"]["used"] == 1


@pytest.mark.parametrize("resource_cli", ["golden-zero"], indirect=True)
def test_rest_changed_etag_replaces_body_before_next_304(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    first = invoke("api", ENDPOINT)
    (trace.parent / "api-version").write_text("2", encoding="utf-8")
    second = invoke("api", ENDPOINT)
    third = invoke("api", ENDPOINT)
    assert first.returncode == second.returncode == third.returncode == 0
    assert json.loads(first.stdout)["fixture_version"] == "1"
    assert json.loads(second.stdout)["fixture_version"] == "2"
    assert third.stdout == second.stdout
    assert calls(trace)[1]["validator"] != calls(trace)[2]["validator"]
    assert len(calls(trace)) == 3


@pytest.mark.parametrize("resource_cli", ["golden-zero"], indirect=True)
def test_rest_missing_validator_uses_full_gets(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    (trace.parent / "api-no-etag").touch()
    first = invoke("api", ENDPOINT)
    second = invoke("api", ENDPOINT)
    assert first.returncode == second.returncode == 0
    assert first.stdout == second.stdout
    assert len(calls(trace)) == 2
    assert all(call["validator"] is None for call in calls(trace))


@pytest.mark.parametrize("resource_cli", ["golden"], indirect=True)
def test_rest_no_store_and_ranges_never_reuse_body(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    for _ in range(2):
        response = invoke("api", "repos/o/r/actions/workflows/7")
        assert response.returncode == 0, response.stderr
    assert len(calls(trace)) == 2, "response no-store forbids retention"
    for _ in range(2):
        response = invoke("api", ENDPOINT, "--header", "Range: bytes=0-99")
        assert response.returncode == 0, response.stderr
    assert len(calls(trace)) == 4
    assert all(call["range"] == "bytes=0-99" for call in calls(trace)[2:])


@pytest.mark.parametrize("resource_cli", ["golden"], indirect=True)
def test_rest_representations_parameters_and_methods_do_not_collide(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    for representation in ("application/x-fixture-a", "application/x-fixture-b"):
        response = invoke("api", ENDPOINT, "--header", "Accept: " + representation)
        assert response.returncode == 0, response.stderr
        assert json.loads(response.stdout)["fixture_accept"] == representation
    for per_page in ("2", "3"):
        response = invoke(
            "api",
            "repos/o/r/actions/runs",
            "--method",
            "GET",
            "-F",
            "per_page=" + per_page,
        )
        assert response.returncode == 0, response.stderr
        assert len(json.loads(response.stdout)["workflow_runs"]) == int(per_page)
    head = invoke("api", ENDPOINT, "--method", "HEAD")
    assert head.returncode == 0, head.stderr
    assert len(calls(trace)) == 5


@pytest.mark.parametrize("resource_cli", ["golden"], indirect=True)
def test_rest_jq_template_share_raw_body_and_match_real_gh(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    jq_args = ("api", ENDPOINT, "--jq", ".workflow_runs[0].id")
    template_args = ("api", ENDPOINT, "--template", "{{(index .workflow_runs 0).id}}")
    jq_golden = invoke(*jq_args, direct=True)
    template_golden = invoke(*template_args, direct=True)
    before = len(calls(trace))
    for args, golden in ((jq_args, jq_golden), (template_args, template_golden)):
        actual = invoke(*args)
        assert (actual.returncode, actual.stdout, actual.stderr) == (
            golden.returncode,
            golden.stdout,
            golden.stderr,
        )
    assert len(calls(trace)) == before + 1


@pytest.mark.parametrize("resource_cli", ["golden"], indirect=True)
def test_rest_errors_preserve_native_body_status_and_do_not_cache(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("api", "fixture/error")
    golden = invoke(*args, direct=True)
    assert golden.returncode == 1
    for _ in range(2):
        actual = invoke(*args)
        assert (actual.returncode, actual.stdout, actual.stderr) == (
            golden.returncode,
            golden.stdout,
            golden.stderr,
        )
    assert len(calls(trace)) == 3


@pytest.mark.parametrize("resource_cli", ["golden"], indirect=True)
def test_rest_rate_limit_is_always_measured_and_pagination_remains_native(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    for _ in range(2):
        measurement = invoke("api", "rate_limit")
        assert measurement.returncode == 0, measurement.stderr
    assert len(calls(trace)) == 2
    args = ("api", "repos/o/r/actions/runs?per_page=100", "--paginate", "--slurp")
    golden = invoke(*args, direct=True)
    actual = invoke(*args)
    assert (actual.returncode, actual.stdout, actual.stderr) == (
        golden.returncode,
        golden.stdout,
        golden.stderr,
    )
    assert len(json.loads(actual.stdout)) == 2
    assert len(calls(trace)) == 6, "each paginated native call must retain both pages"


def test_portable_rest_retention_uses_the_same_protobuf_route(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    first = invoke("api", "fixture/body")
    second = invoke("api", "fixture/body", "-XGET")
    assert first.returncode == second.returncode == 0, (first.stderr, second.stderr)
    assert json.loads(first.stdout) == {"number": 42, "title": "portable REST fixture"}
    assert first.stdout == second.stdout
    assert len(calls(trace)) == 1


@pytest.mark.parametrize("resource_cli", ["golden"], indirect=True)
def test_request_max_age_zero_validates_instead_of_using_the_fresh_window(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("api", ENDPOINT, "--header", "Cache-Control: max-age=0")
    first = invoke(*args)
    second = invoke(*args)
    assert first.returncode == second.returncode == 0, (first.stderr, second.stderr)
    assert first.stdout == second.stdout
    assert len(calls(trace)) == 2
    assert calls(trace)[1]["validator"] is not None
