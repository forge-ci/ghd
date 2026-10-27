"""ghx parity #9: installed CLI reads share resource data across projections.

The executable fixture emits synthetic gh-shaped JSON and records cold reads.
All credentials, state and subprocesses belong to the test; no GitHub API is
used. The renderer branch only accepts an authenticated loopback endpoint.
"""

from __future__ import annotations

import json
import shutil
import subprocess
from collections.abc import Callable
from pathlib import Path

import pytest

from ghd.platforms import is_windows

ResourceCli = tuple[Callable[..., subprocess.CompletedProcess[str]], Path]


def test_issue_superset_and_subset_share_one_cold_fetch(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    first = invoke("issue", "view", "42", "-R", "o/r", "--json", "number,title,state")
    second = invoke("issue", "view", "42", "--json", "state,title", "--repo", "o/r")
    assert first.returncode == second.returncode == 0, (first.stderr, second.stderr)
    assert json.loads(first.stdout) == {
        "number": 42,
        "title": "all 42",
        "state": "OPEN",
    }
    assert json.loads(second.stdout) == {"title": "all 42", "state": "OPEN"}
    calls = [
        json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()
    ]
    assert len(calls) == 1, (
        "equivalent resource reads must not each execute gh upstream"
    )


def cold_calls(trace: Path) -> list[list[str]]:
    return [json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()]


def test_collection_limits_share_rows_and_filters_isolate(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("issue", "list", "-R", "o/r", "--json", "number,title")
    first = invoke(*args, "--label", "ready", "--limit", "2")
    second = invoke(*args, "--label", "ready", "--limit", "4")
    other = invoke(*args, "--label", "blocked", "--limit", "1")
    assert first.returncode == second.returncode == other.returncode == 0
    assert len(json.loads(first.stdout)) == 2
    assert len(json.loads(second.stdout)) == 4
    assert json.loads(other.stdout) == [{"number": 1, "title": "blocked 1"}]
    assert len(cold_calls(trace)) == 2


def test_nested_pr_checks_and_pending_exit_are_retained(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("pr", "view", "42", "-R", "o/r")
    first = invoke(*args, "--json", "reviews,statusCheckRollup,author")
    second = invoke(*args, "--json", "author,reviews")
    assert first.returncode == second.returncode == 0, (first.stderr, second.stderr)
    assert json.loads(second.stdout) == {
        "author": None,
        "reviews": [{"state": "APPROVED", "author": {"login": "reviewer"}}],
    }
    assert json.loads(first.stdout)["statusCheckRollup"][0]["conclusion"] == "SUCCESS"
    for fields in ("name,bucket,workflow", "bucket,name"):
        checks = invoke("pr", "checks", "42", "-R", "o/r", "--json", fields)
        assert checks.returncode == 8, checks.stderr
        assert json.loads(checks.stdout)[0]["bucket"] == "pending"
    assert len(cold_calls(trace)) == 2


def test_run_attempts_and_unknown_fields_keep_native_status(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("run", "view", "42", "-R", "o/r", "--json", "attempt,jobs")
    first = invoke(*args, "--attempt", "1")
    second = invoke(*args, "--attempt", "2")
    again = invoke(*args, "--attempt", "1")
    assert first.returncode == second.returncode == again.returncode == 0
    assert (
        json.loads(first.stdout)
        == json.loads(again.stdout)
        == {"attempt": 1, "jobs": []}
    )
    assert json.loads(second.stdout) == {"attempt": 2, "jobs": []}
    for _ in range(2):
        unsupported = invoke(
            "issue", "view", "42", "-R", "o/r", "--json", "futureField"
        )
        assert unsupported.returncode == 1
        assert unsupported.stderr == "Unknown JSON field: futureField\n"
    assert len(cold_calls(trace)) == 4, "failed schema reads must not be cached"


def test_jq_and_template_render_from_shared_data(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    args = ("issue", "view", "42", "-R", "o/r", "--json", "number,title")
    initial = invoke(*args)
    jq = invoke(*args, "--jq", ".title")
    template = invoke(*args, "--template", "{{.title}}")
    assert initial.returncode == jq.returncode == template.returncode == 0, (
        jq.stderr,
        template.stderr,
    )
    assert jq.stdout == "all 42\n"
    assert template.stdout == "all 42"
    assert len(cold_calls(trace)) == 1
    assert not list((trace.parent / "state").glob("identities-v1/*/renders/*"))


@pytest.mark.skipif(
    shutil.which("gh") is None, reason="real gh renderer requires gh on PATH"
)
def test_real_gh_jq_supports_nested_data_nulls_and_errors(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("pr", "view", "42", "-R", "o/r", "--json", "author,reviews")
    jq = invoke(*args, "--jq", '[.author // "none", .reviews[].state]')
    assert jq.returncode == 0, jq.stderr
    assert json.loads(jq.stdout) == ["none", "APPROVED"]
    invalid = invoke(*args, "--jq", ".[")
    assert invalid.returncode == 1
    assert invalid.stderr
    ordinary = invoke(*args)
    assert ordinary.returncode == 0
    assert json.loads(ordinary.stdout)["author"] is None
    assert len(cold_calls(trace)) == 1


@pytest.mark.skipif(
    is_windows() or shutil.which("gh") is None, reason="real gh local Unix transport"
)
@pytest.mark.parametrize("resource_cli", ["golden"], indirect=True)
@pytest.mark.parametrize(
    "args",
    [
        ("issue", "view", "42", "--json", "number,title,author,labels"),
        ("issue", "list", "--json", "number,title,author", "--limit", "1"),
        ("pr", "view", "42", "--json", "reviews,statusCheckRollup"),
        ("pr", "list", "--json", "number,title,reviews", "--limit", "1"),
        ("pr", "checks", "42"),
        ("pr", "checks", "42", "--json", "name,bucket,state,workflow"),
        (
            "run",
            "list",
            "--json",
            "databaseId,attempt,name,conclusion,startedAt",
            "--limit",
            "1",
        ),
        (
            "run",
            "view",
            "42",
            "--json",
            "databaseId,attempt,conclusion,startedAt,workflowName",
        ),
        (
            "run",
            "list",
            "--json",
            "databaseId,attempt,conclusion,startedAt,workflowName",
            "--limit",
            "1",
        ),
        ("run", "list", "--json", "databaseId,workflowName", "--limit", "101"),
    ],
)
def test_real_gh_golden_output_and_status(
    resource_cli: ResourceCli, args: tuple[str, ...]
) -> None:
    invoke, trace = resource_cli
    args = (*args, "--repo", "o/r")
    golden = invoke(*args, direct=True)
    assert golden.returncode in (0, 8), golden.stderr
    first = invoke(*args)
    assert first.returncode == golden.returncode, first.stderr
    assert first.stdout == golden.stdout
    assert first.stderr == golden.stderr
    if "--limit" in args and args[args.index("--limit") + 1] == "101":
        assert len(json.loads(first.stdout)) == 101, (
            "native pagination must cross the 100-row boundary"
        )
    count = len(trace.read_text(encoding="utf-8").splitlines())
    second = invoke(*args)
    assert (second.returncode, second.stdout, second.stderr) == (
        golden.returncode,
        golden.stdout,
        golden.stderr,
    )
    assert len(trace.read_text(encoding="utf-8").splitlines()) == count
    binding = invoke(
        python_code=(
            "import json,sys; from ghd.bindings import gh_read; "
            f"r=gh_read(json.loads({json.dumps(args)!r})); "
            "sys.stdout.buffer.write(r.stdout); sys.stderr.buffer.write(r.stderr); "
            "raise SystemExit(r.exit_code)"
        )
    )
    assert (binding.returncode, binding.stdout, binding.stderr) == (
        golden.returncode,
        golden.stdout,
        golden.stderr,
    )
    assert len(trace.read_text(encoding="utf-8").splitlines()) == count
    if args[:2] == ("pr", "checks") and "--json" not in args:
        assert golden.returncode == 8, "fixture must exercise pending checks"


@pytest.mark.parametrize(
    "args",
    [
        ("pr", "status", "-R", "o/r"),
        ("pr", "diff", "42", "-R", "o/r"),
        ("issue", "status", "-R", "o/r"),
        ("repo", "view", "o/r"),
        ("repo", "list", "owner"),
        ("workflow", "list", "-R", "o/r"),
        ("workflow", "view", "build", "--yaml", "-R", "o/r"),
        ("release", "list", "-R", "o/r"),
        ("release", "view", "v1", "-R", "o/r"),
        ("search", "repos", "fixture"),
        ("search", "issues", "fixture"),
        ("search", "prs", "fixture"),
        ("search", "commits", "fixture"),
        ("search", "code", "fixture"),
        ("label", "list", "-R", "o/r"),
        ("gist", "list"),
        ("gist", "view", "abcd"),
        ("project", "list", "--owner", "team"),
        ("project", "view", "1", "--owner", "team"),
        ("project", "field-list", "1", "--owner", "team"),
        ("project", "item-list", "1", "--owner", "team"),
        ("cache", "list", "-R", "o/r"),
        ("ruleset", "list", "-R", "o/r"),
        ("ruleset", "view", "1", "-R", "o/r"),
        ("ruleset", "check", "main", "-R", "o/r"),
        ("org", "list"),
        ("secret", "list", "-R", "o/r"),
        ("variable", "list", "-R", "o/r"),
        ("variable", "get", "NAME", "-R", "o/r"),
    ],
)
def test_other_allowlisted_reads_have_exact_output_caching(
    resource_cli: ResourceCli, args: tuple[str, ...]
) -> None:
    invoke, trace = resource_cli
    first = invoke(*args)
    second = invoke(*args)
    assert first.returncode == second.returncode == 0, (first.stderr, second.stderr)
    assert first.stdout == second.stdout
    assert json.loads(first.stdout)["fixture_args"][:2] == list(args[:2])
    assert len(cold_calls(trace)) == 1, (
        "this family must cache rather than only pass through"
    )


@pytest.mark.parametrize("name", ["GH_DEBUG", "DEBUG"])
def test_debug_reads_preserve_diagnostics_and_never_enter_the_cache(
    resource_cli: ResourceCli, name: str
) -> None:
    invoke, trace = resource_cli
    args = ("issue", "view", "42", "-R", "o/r", "--json", "title")
    for _ in range(2):
        response = invoke(*args, context={name: "api"})
        assert response.returncode == 0, response.stderr
        assert response.stderr == "fixture API diagnostics\n"
    assert len(cold_calls(trace)) == 2
    ordinary = invoke(*args)
    again = invoke(*args)
    assert ordinary.returncode == again.returncode == 0
    assert ordinary.stderr == again.stderr == ""
    assert len(cold_calls(trace)) == 3
    binding = invoke(
        context={name: "api"},
        python_code=(f"from ghd.bindings import gh_read; gh_read({list(args)!r})"),
    )
    assert binding.returncode == 1
    assert "debug/forced-terminal" in binding.stderr
    assert len(cold_calls(trace)) == 3


@pytest.mark.skipif(
    is_windows() or shutil.which("gh") is None, reason="real gh local Unix transport"
)
@pytest.mark.parametrize("resource_cli", ["golden-zero"], indirect=True)
def test_collection_etag_304_reuses_rows_through_caller_transport(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = (
        "run",
        "list",
        "-R",
        "o/r",
        "--json",
        "databaseId,name,attempt",
        "--limit",
        "1",
    )
    golden = invoke(*args, direct=True)
    assert golden.returncode == 0, golden.stderr
    before = len(trace.read_text(encoding="utf-8").splitlines())
    first = invoke(*args, context={"GHD_DEBUG": "1"})
    second = invoke(*args, context={"GHD_DEBUG": "1"})
    assert first.returncode == second.returncode == 0, (first.stderr, second.stderr)
    assert first.stdout == second.stdout == golden.stdout
    assert first.stderr == "ghd: fetched\n"
    assert second.stderr == "ghd: revalidated\n"
    assert len(trace.read_text(encoding="utf-8").splitlines()) == before + 2
