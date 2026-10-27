"""Shared installed CLI, private daemon and local API fixtures.

Each test owns its credentials, configuration, state and processes. The real
GitHub CLI can use the fixture's private Unix API transport for golden output.
"""

from __future__ import annotations

import http.server
import json
import os
import re
import shlex
import shutil
import socketserver
import subprocess
import sys
import threading
import time
from collections.abc import Callable, Iterator
from pathlib import Path
from urllib.parse import parse_qs, urlencode, urlsplit

import pytest

from ghd.platforms import cli_binary_name, is_windows

FAKE_GH = r"""
import json, os, sys, urllib.request
sys.stdout.reconfigure(encoding="utf-8", newline="\n")
args = sys.argv[1:]
if args and args[0] == "api":
    if REAL_GH is not None:
        os.execv(REAL_GH, [REAL_GH, *args])
    endpoint = args[1]
    if not endpoint.startswith("http://127.0.0.1:"):
        raise SystemExit("fixture renderer refuses non-loopback requests")
    token = os.environ["GH_ENTERPRISE_TOKEN"]
    request = urllib.request.Request(endpoint, headers={"Authorization": "token " + token})
    with urllib.request.urlopen(request, timeout=3) as response:
        data = json.load(response)
    if "--jq" in args:
        expression = args[args.index("--jq") + 1]
        if expression not in (".title", ".state", ".number"):
            raise SystemExit("fixture filter not declared")
        result = data[expression[1:]]
        print(result if isinstance(result, str) else json.dumps(result))
    elif "--template" in args:
        template = args[args.index("--template") + 1]
        if template != "{{.title}}":
            raise SystemExit("fixture template not declared")
        print(data["title"], end="")
    else:
        print(json.dumps(data, sort_keys=True, separators=(",", ":")))
    raise SystemExit(0)
if os.environ.get("GH_DEBUG") or os.environ.get("DEBUG"):
    print("fixture API diagnostics", file=sys.stderr)
with open(TRACE_PATH, "a", encoding="utf-8") as trace:
    trace.write(json.dumps(args) + "\n")
kind, action = args[:2]
if kind not in ("issue", "pr", "run") or action not in ("view", "list", "checks"):
    print(json.dumps({"fixture_args": args}, sort_keys=True, separators=(",", ":")))
    raise SystemExit(0)
fields = args[args.index("--json") + 1].split(",")
if "futureField" in fields:
    print("Unknown JSON field: futureField", file=sys.stderr)
    raise SystemExit(1)
label = args[args.index("--label") + 1] if "--label" in args else "all"
attempt = int(args[args.index("--attempt") + 1]) if "--attempt" in args else 1

def row(number):
    return {"number": number, "databaseId": number, "id": "I_fixture", "title": f"{label} {number}",
            "state": "OPEN", "body": "synthetic fixture", "url": "https://github.com/o/r/issues/42",
            "author": None, "labels": [], "assignees": [], "comments": [], "milestone": None,
            "reviews": [{"state": "APPROVED", "author": {"login": "reviewer"}}],
            "statusCheckRollup": [{"name": "build", "status": "COMPLETED", "conclusion": "SUCCESS"}],
            "attempt": attempt, "jobs": [], "name": "build", "bucket": "pending", "workflow": "CI"}

def project(data):
    return {field: data.get(field) for field in fields}

if action == "list":
    data = [project(row(number)) for number in range(1, 6)]
elif action == "checks":
    data = [project(row(42))]
else:
    data = project(row(int(args[2])))
print(json.dumps(data, sort_keys=True, separators=(",", ":")))
raise SystemExit(8 if action == "checks" else 0)
"""


ResourceCli = tuple[Callable[..., subprocess.CompletedProcess[str]], Path]


@pytest.fixture(name="resource_cli")
def _resource_cli(
    tmp_path: Path, request: pytest.FixtureRequest
) -> Iterator[ResourceCli]:
    binary = Path(sys.executable).parent / cli_binary_name()
    assert binary.is_file(), "run against the installed soldr-built wheel"
    script = tmp_path / "fixture_gh.py"
    trace = tmp_path / "cold_reads.jsonl"
    script.write_text(
        "TRACE_PATH = "
        + repr(str(trace))
        + "\nREAL_GH = "
        + repr(shutil.which("gh"))
        + "\n"
        + FAKE_GH,
        encoding="utf-8",
    )
    if is_windows():
        wrapper = tmp_path / "gh.cmd"
        wrapper.write_text(f'@"{sys.executable}" "{script}" %*\r\n', encoding="utf-8")
    else:
        wrapper = tmp_path / "gh"
        wrapper.write_text(
            f"#!/bin/sh\nexec {shlex.quote(sys.executable)} "
            f'{shlex.quote(str(script))} "$@"\n',
            encoding="utf-8",
        )
        wrapper.chmod(0o700)
    state = tmp_path / "state"
    config = tmp_path / "gh-config"
    config.mkdir(mode=0o700)
    fixture_home = tmp_path / "home"
    fixture_home.mkdir(mode=0o700)
    env = dict(os.environ)
    for name in (
        "GHD_PASSTHROUGH",
        "GHD_DEBUG",
        "GH_DEBUG",
        "DEBUG",
        "GH_FORCE_TTY",
        "CLICOLOR_FORCE",
        "GH_REPO",
    ):
        env.pop(name, None)
    env.update(
        GHD_GH_PATH=str(wrapper),
        GHD_EXE=str(binary),
        GHD_STATE_DIR=str(state),
        GHD_FRESH_MS="60000",
        GH_CONFIG_DIR=str(config),
        HOME=str(fixture_home),
        USERPROFILE=str(fixture_home),
        XDG_CACHE_HOME=str(tmp_path / "gh-cache"),
        GH_HOST="github.com",
        GH_TOKEN="resource-fixture-token",
        GH_ENTERPRISE_TOKEN="resource-fixture-token",
        HTTP_PROXY="http://127.0.0.1:0",
        HTTPS_PROXY="http://127.0.0.1:0",
        NO_PROXY="127.0.0.1,localhost",
    )
    if getattr(request, "param", None) in ("golden", "golden-zero"):
        if request.param == "golden-zero":
            env["GHD_FRESH_MS"] = "0"
        socket, trace = request.getfixturevalue("mock_github")
        (config / "config.yml").write_text(
            "http_unix_socket: " + json.dumps(str(socket)) + "\n", encoding="utf-8"
        )
        env["GHD_GH_PATH"] = str(shutil.which("gh"))
    with subprocess.Popen(
        [str(binary), "daemon"],
        env=env,
        cwd=tmp_path,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    ) as daemon:
        try:
            deadline = time.monotonic() + 10
            while not list(state.glob("identities-v1/*/endpoint")):
                assert daemon.poll() is None, (
                    "fixture daemon exited before publishing an endpoint"
                )
                assert time.monotonic() < deadline, "fixture daemon startup timed out"
                time.sleep(0.01)

            def invoke(
                *args: str,
                direct: bool = False,
                python_code: str | None = None,
                context: dict[str, str] | None = None,
            ) -> subprocess.CompletedProcess[str]:
                command = (
                    [str(shutil.which("gh")), *args]
                    if direct
                    else [str(binary), "gh", *args]
                )
                if python_code is not None:
                    command = [sys.executable, "-c", python_code]
                return subprocess.run(
                    command,
                    env=env | (context or {}),
                    cwd=tmp_path,
                    capture_output=True,
                    text=True,
                    timeout=15,
                    check=False,
                )

            yield invoke, trace
        finally:
            stop_daemon(daemon)


def stop_daemon(daemon: subprocess.Popen[bytes]) -> None:
    if daemon.poll() is None:
        daemon.terminate()
    try:
        daemon.wait(timeout=10)
    except subprocess.TimeoutExpired:
        daemon.kill()
        daemon.wait(timeout=10)
    for stream in (daemon.stdout, daemon.stderr):
        if stream is not None:
            stream.close()


@pytest.fixture(name="mock_github")
def _mock_github(tmp_path: Path) -> Iterator[tuple[Path, Path]]:
    """The real gh transport goes only to this private local API fixture."""
    socket = tmp_path / "api.sock"
    trace = tmp_path / "api-calls.jsonl"
    context = {
        "__typename": "CheckRun",
        "name": "build",
        "status": "IN_PROGRESS",
        "conclusion": "",
        "detailsUrl": "https://example.invalid/check",
        "startedAt": "2026-01-01T00:00:00Z",
        "completedAt": "2026-01-01T00:01:00Z",
        "checkSuite": {"workflowRun": {"event": "push", "workflow": {"name": "CI"}}},
    }
    rollup = {
        "nodes": [
            {
                "commit": {
                    "oid": "abc",
                    "statusCheckRollup": {"contexts": {"nodes": [context]}},
                }
            }
        ]
    }
    issue = {
        "id": "I_fixture",
        "number": 42,
        "title": "golden issue",
        "state": "OPEN",
        "body": "fixture",
        "url": "https://example.invalid/issues/42",
        "author": None,
        "labels": {"nodes": []},
        "assignees": {"nodes": []},
        "comments": {"nodes": []},
    }
    pull = {
        **issue,
        "id": "PR_fixture",
        "reviews": {
            "nodes": [
                {
                    "state": "APPROVED",
                    "author": {"login": "reviewer"},
                    "body": "approved",
                }
            ]
        },
        "statusCheckRollup": rollup,
    }
    runs = [
        {
            "id": n,
            "run_number": n,
            "run_attempt": 2,
            "workflow_id": 7,
            "name": "run name",
            "display_title": "golden run",
            "status": "queued",
            "conclusion": None,
            "head_branch": "main",
            "head_sha": "abc",
            "event": "push",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "run_started_at": None,
            "html_url": f"https://example.invalid/runs/{n}",
        }
        for n in range(42, 174)
    ]

    handler = mock_handler(trace, issue, pull, runs, rollup)

    # Unix transport is an official gh test/development option. This fixture's
    # tests select Unix hosts through ghd's existing platform facade.
    with socketserver.UnixStreamServer(str(socket), handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield socket, trace
        finally:
            server.shutdown()
            thread.join(timeout=5)


def mock_handler(
    trace: Path,
    issue: dict[str, object],
    pull: dict[str, object],
    runs: list[dict[str, object]],
    rollup: dict[str, object],
) -> type[http.server.BaseHTTPRequestHandler]:
    return type(
        "FixtureHandler",
        (MockGithubHandler,),
        {"fixture": (trace, issue, pull, runs, rollup)},
    )


class MockGithubHandler(http.server.BaseHTTPRequestHandler):
    fixture: tuple[
        Path,
        dict[str, object],
        dict[str, object],
        list[dict[str, object]],
        dict[str, object],
    ]

    def log_message(self, *_args: object, **_kwargs: object) -> None:
        pass

    def reply(
        self, payload: object, *, collection: bool = False, next_path: str | None = None
    ) -> None:
        if collection and self.headers.get("If-None-Match") == '"runs-fixture"':
            self.send_response(304)
            self.send_header("ETag", '"runs-fixture"')
            self.end_headers()
            return
        encoded = json.dumps(payload).encode()
        self.send_response(200)
        if collection:
            self.send_header("ETag", '"runs-fixture"')
        else:
            self.send_header("Cache-Control", "no-store")
        if next_path:
            self.send_header("Link", f'<https://api.github.com{next_path}>; rel="next"')
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def record(self, query: str | None = None) -> None:
        with self.fixture[0].open("a", encoding="utf-8") as output:
            output.write(json.dumps({"path": self.path, "query": query}) + "\n")

    def do_POST(self) -> None:  # pylint: disable=invalid-name
        _, issue, pull, _, rollup = self.fixture
        payload = json.loads(
            self.rfile.read(int(self.headers.get("Content-Length", "0")))
        )
        self.record(payload.get("query"))
        if "__type(" in payload.get("query", ""):
            aliases = re.findall(r"(\w+):\s*__type\(", payload["query"])
            self.reply({"data": {alias: {"fields": []} for alias in aliases}})
            return
        if "PullRequestByNumber" in payload.get(
            "query", ""
        ) and "headRefName,id" in payload.get("query", ""):
            self.reply(
                {
                    "data": {
                        "repository": {
                            "pullRequest": {
                                "id": "PR_fixture",
                                "number": 42,
                                "headRefName": "main",
                            }
                        }
                    }
                }
            )
            return
        if re.search(r"\bnode\s*\(", payload.get("query", "")):
            self.reply({"data": {"node": {"statusCheckRollup": rollup}}})
            return
        self.reply(
            {
                "data": {
                    "repository": {
                        "hasIssuesEnabled": True,
                        "issue": issue,
                        "pullRequest": pull,
                        "issues": {
                            "nodes": [issue],
                            "pageInfo": {"hasNextPage": False},
                        },
                        "pullRequests": {
                            "nodes": [pull],
                            "pageInfo": {"hasNextPage": False},
                        },
                    },
                    "node": pull,
                }
            }
        )

    def do_GET(self) -> None:  # pylint: disable=invalid-name
        runs = self.fixture[3]
        self.record()
        if "/actions/runs?" in self.path:
            target = urlsplit(self.path)
            query = {key: values[0] for key, values in parse_qs(target.query).items()}
            per_page, page = (
                int(query.get("per_page", "30")),
                int(query.get("page", "1")),
            )
            rows = runs[(page - 1) * per_page : page * per_page]
            next_path = (
                (target.path + "?" + urlencode(query | {"page": str(page + 1)}))
                if page * per_page < len(runs)
                else None
            )
            self.reply(
                {"total_count": len(runs), "workflow_runs": rows},
                collection=True,
                next_path=next_path,
            )
        elif "/actions/workflows?" in self.path:
            self.reply(
                {
                    "total_count": 1,
                    "workflows": [
                        {"id": 7, "name": "workflow metadata", "state": "active"}
                    ],
                }
            )
        elif "/actions/workflows/7" in self.path:
            self.reply({"id": 7, "name": "workflow metadata"})
        elif "/actions/runs/42" in self.path:
            self.reply(runs[0])
        else:
            raise AssertionError(f"undeclared API request: {self.path}")
