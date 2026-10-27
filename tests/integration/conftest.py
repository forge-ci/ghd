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
import tempfile
import threading
import time
from collections.abc import Callable, Iterator
from contextlib import ExitStack, contextmanager
from pathlib import Path
from typing import Literal
from urllib.parse import parse_qs, urlencode, urlsplit

import pytest

from ghd.platforms import cli_binary_name, is_windows

FAKE_GH = r"""
import json, os, subprocess, sys, time, urllib.request
from pathlib import Path
sys.stdout.reconfigure(encoding="utf-8", newline="\n")
args = sys.argv[1:]
if args[:2] == ["api", "graphql"] and any("GhdSnapshotV1" in arg for arg in args):
    with open(TRACE_PATH, "a", encoding="utf-8") as trace:
        trace.write(json.dumps(args) + "\n")
    fixture_root = Path(TRACE_PATH).parent
    if (fixture_root / "snapshot-fail").exists():
        print('HTTP/1.1 503 unavailable\n\n{"message":"fixture unavailable"}')
        print('fixture snapshot unavailable', file=sys.stderr)
        raise SystemExit(1)
    def option(name, default):
        return next((arg.split("=", 1)[1] for arg in args if arg.startswith(name + "=")), default)
    def connection(nodes):
        return {"nodes": nodes, "totalCount": len(nodes), "pageInfo": {
            "hasNextPage": False, "hasPreviousPage": False,
            "startCursor": "start" if nodes else None, "endCursor": "end" if nodes else None}}
    def rollup():
        return {"state": "SUCCESS", "contexts": connection([
            {"__typename": "CheckRun", "databaseId": 7, "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS"},
            {"__typename": "StatusContext", "context": "external", "state": "SUCCESS", "targetUrl": "https://example.invalid/status"}])}
    title = (fixture_root / "snapshot-title").read_text(encoding="utf-8") if (fixture_root / "snapshot-title").exists() else "snapshot title"
    pr = {"number": 42, "title": title, "headRefOid": "abc", "headRefName": "feature",
          "baseRefOid": "main-oid", "baseRefName": "main", "state": "OPEN", "isDraft": False,
          "files": connection([{"path": "src/app.rs", "additions": 1, "deletions": 0}]),
          "reviews": connection([{"state": "APPROVED", "author": {"login": "reviewer"}, "commit": {"oid": "abc"}}]),
          "commits": connection([{"commit": {"oid": "abc", "committedDate": "2026-01-01T00:00:00Z", "statusCheckRollup": rollup()}}])}
    repo = {"nameWithOwner": option("owner", "o") + "/" + option("name", "r"), "isPrivate": False,
            "defaultBranchRef": {"name": "main", "target": {"oid": "main-oid", "statusCheckRollup": rollup()}},
            "pullRequests": connection([pr])}
    if (fixture_root / "snapshot-empty").exists():
        repo["defaultBranchRef"] = None
        repo["pullRequests"] = connection([])
    if (fixture_root / "snapshot-truncated").exists():
        pr["reviews"]["pageInfo"].update(hasPreviousPage=True, startCursor="older-reviews")
        pr["reviews"]["totalCount"] = 101
    if (fixture_root / "snapshot-root-truncated").exists():
        repo["pullRequests"]["pageInfo"].update(hasNextPage=True, endCursor="more-prs")
        repo["pullRequests"]["totalCount"] = 101
    if (fixture_root / "snapshot-missing-pages").exists():
        del repo["pullRequests"]["pageInfo"]
    if (fixture_root / "snapshot-head-mismatch").exists():
        pr["headRefOid"] = "different-head"
    if (fixture_root / "snapshot-no-checks").exists():
        pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = None
    print('HTTP/1.1 200 OK')
    policy_file = fixture_root / "snapshot-cache-control"
    print('Cache-Control: ' + (policy_file.read_text() if policy_file.exists() else 'private, max-age=60'))
    print('X-RateLimit-Resource: graphql\nX-RateLimit-Remaining: 5000\nX-RateLimit-Reset: 4102444800')
    if (fixture_root / "snapshot-stale").exists():
        print('X-Ghgate-Stale: true\nAge: 120')
    if (fixture_root / "snapshot-stale-no-age").exists():
        print('X-Ghgate-Stale: true')
    age_file = fixture_root / "snapshot-age"
    if age_file.exists():
        print('Age: ' + age_file.read_text())
    print()
    body = {"data": {"repository": repo}}
    if (fixture_root / "snapshot-partial-error").exists():
        body["errors"] = [{"message": "fixture GraphQL partial error"}]
    print(json.dumps(body))
    raise SystemExit(0)
collection = len(args) > 1 and args[0] == "api" and args[1].startswith("repos/o/r/actions/runs?")
if args[:2] == ["api", "fixture/body"] or collection:
    with open(TRACE_PATH, "a", encoding="utf-8") as trace:
        trace.write(json.dumps(args) + "\n")
    if Path(TRACE_PATH).with_name("revalidation-fails").exists():
        print("fixture origin unavailable", file=sys.stderr)
        raise SystemExit(1)
    status_file = Path(TRACE_PATH).with_name("provider-status")
    status = int(status_file.read_text()) if status_file.exists() else 200
    conditional = status == 200 and any(word.startswith("If-None-Match:") for word in args)
    status = 304 if conditional else status
    print(f'HTTP/1.1 {status} fixture')
    retry_file = Path(TRACE_PATH).with_name("retry-after")
    if retry_file.exists():
        print('Retry-After: ' + retry_file.read_text())
    print('ETag: "portable-v1"')
    remaining_file = Path(TRACE_PATH).with_name("quota-remaining")
    remaining = remaining_file.read_text() if remaining_file.exists() else "5000"
    print('X-RateLimit-Resource: core')
    print('X-RateLimit-Remaining: ' + remaining)
    print('X-RateLimit-Limit: 5000')
    print('X-RateLimit-Reset: 4102444800')
    policy = Path(TRACE_PATH).with_name("collection-policy")
    print('Cache-Control: ' + (policy.read_text() if collection and policy.exists() else 'private, max-age=60'))
    print()
    if status >= 400:
        print('{"message":"fixture provider error"}')
        print(f'fixture provider error (HTTP {status})', file=sys.stderr)
        raise SystemExit(1)
    if not conditional:
        if collection:
            print('{"workflow_runs":[{"id":42,"run_attempt":1,"name":"build","status":"completed","conclusion":"success"}]}')
        else:
            print('{"number":42,"title":"portable REST fixture"}')
    raise SystemExit(0)
if args and args[0] == "api":
    if REAL_GH is not None:
        # Keep the supervised wrapper alive until its renderer exits. Windows
        # execv spawns/replaces differently from a Unix exec and can let the
        # wrapper's lifetime end before output is collected.
        raise SystemExit(subprocess.run([REAL_GH, *args], check=False).returncode)
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
hold = Path(TRACE_PATH).with_name("operation-hold")
if hold.exists():
    Path(TRACE_PATH).with_name("operation-started").touch()
    deadline = time.monotonic() + 5
    while hold.read_text(encoding="utf-8") != "released":
        if time.monotonic() >= deadline:
            raise SystemExit(1)
        time.sleep(0.01)
kind, action = args[:2]
if kind == "auth" and action == "fixture-exit":
    print(json.dumps(args))
    print("fixture delegated error", file=sys.stderr)
    raise SystemExit(23)
if action == "edit":
    raise SystemExit(0)
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
            "attempt": attempt, "status": "completed", "headSha": "abc", "jobs": [], "name": "build", "bucket": "pending", "workflow": "CI"}

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


def apply_fixture_overrides(
    env: dict[str, str], request: pytest.FixtureRequest
) -> None:
    """Only test-owned child environments receive per-case configuration."""
    overrides = getattr(request, "param", None)
    if isinstance(overrides, dict):
        env.update(overrides)


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
        "GHD_CALLER_ID",
    ):
        env.pop(name, None)
    env.update(
        GHD_GH_PATH=str(wrapper),
        GHD_EXE=str(binary),
        GHD_STATE_DIR=str(state),
        GHD_FRESH_MS="60000",
        GHD_CACHE_MAX_ENTRIES="4096",
        GHD_CACHE_MAX_BYTES="67108864",
        GHD_CACHE_DISK="0",
        GHD_RATE_RESERVE="0",
        GHD_LOG_ARGS="0",
        GHD_CACHE_TTLS="{}",
        GHD_CACHE_SETTLED_MS="2000",
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
    apply_fixture_overrides(env, request)
    if getattr(request, "param", None) in ("golden", "golden-zero"):
        if request.param == "golden-zero":
            env["GHD_FRESH_MS"] = "0"
        socket, trace = request.getfixturevalue("mock_github")
        (config / "config.yml").write_text(
            "http_unix_socket: " + json.dumps(str(socket)) + "\n", encoding="utf-8"
        )
        env["GHD_GH_PATH"] = str(shutil.which("gh"))
    with ExitStack() as daemons:
        daemon = daemons.enter_context(
            start_fixture_daemon(binary, env, tmp_path, state)
        )

        # Cold delegated launches need an explicit cleanup budget; the other
        # capture/context knobs remain at this single test-owned boundary.
        # pylint: disable-next=too-many-arguments
        def invoke(
            *args: str,
            direct: bool = False,
            native: bool = False,
            python_code: str | None = None,
            context: dict[str, str | None] | None = None,
            daemon_action: Literal["restart", "stop"] | None = None,
            timeout: float = 15,
        ) -> subprocess.CompletedProcess[str]:
            nonlocal daemon
            if daemon_action == "stop":
                stop_daemon(daemon)
            if daemon_action == "restart":
                previous = next(state.glob("identities-v1/*/endpoint")).read_bytes()
                stop_daemon(daemon)
                daemon = daemons.enter_context(
                    start_fixture_daemon(binary, env, tmp_path, state, previous)
                )
            command = (
                [str(shutil.which("gh")), *args]
                if direct
                else [str(binary), "gh", *args]
            )
            if native:
                command = [str(binary), *args]
            if python_code is not None:
                command = [sys.executable, "-c", python_code]
            return subprocess.run(
                command,
                env={
                    key: value
                    for key, value in (env | (context or {})).items()
                    if value is not None
                },
                cwd=tmp_path,
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
            )

        yield invoke, trace


@contextmanager
def start_fixture_daemon(
    binary: Path,
    env: dict[str, str],
    cwd: Path,
    state: Path,
    previous: bytes | None = None,
) -> Iterator[subprocess.Popen[bytes]]:
    with subprocess.Popen(
        [str(binary), "daemon"],
        env=env,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    ) as daemon:
        try:
            deadline = time.monotonic() + 10
            last_error: OSError | None = None
            while True:
                endpoints = list(state.glob("identities-v1/*/endpoint"))
                try:
                    ready = bool(endpoints) and endpoints[0].read_bytes() != previous
                except (FileNotFoundError, PermissionError) as error:
                    # Windows publication/delete sharing can briefly deny an
                    # observer. The existing startup deadline still bounds it.
                    last_error = error
                    ready = False
                if ready:
                    break
                assert daemon.poll() is None, (
                    "fixture daemon exited before publishing an endpoint"
                )
                assert time.monotonic() < deadline, (
                    f"fixture daemon startup timed out: {last_error}"
                )
                time.sleep(0.01)
            yield daemon
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
    if is_windows() or shutil.which("gh") is None:
        pytest.skip("real gh local Unix transport")
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
    # The pytest case directory can exceed macOS's sockaddr_un path limit.
    # A separately owned short temporary directory keeps the transport real.
    with tempfile.TemporaryDirectory(prefix="ga-") as socket_dir:
        socket = Path(socket_dir) / "a"
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

    def fixture_value(self, name: str, default: str) -> str:
        path = self.fixture[0].parent / name
        return path.read_text(encoding="utf-8") if path.exists() else default

    def quota(self, status: int) -> int:
        used = int(self.fixture_value("core-used", "0"))
        if (
            status == 200
            and self.command == "GET"
            and urlsplit(self.path).path != "/rate_limit"
        ):
            used += 1
            (self.fixture[0].parent / "core-used").write_text(
                str(used), encoding="utf-8"
            )
        return used

    def reply(
        self,
        payload: object,
        *,
        collection: bool = False,
        next_path: str | None = None,
        status: int = 200,
    ) -> None:
        etag = '"runs-fixture-' + self.fixture_value("api-version", "1") + '"'
        validator = collection and not (self.fixture[0].parent / "api-no-etag").exists()
        unchanged = validator and self.headers.get("If-None-Match") == etag
        status = 304 if unchanged else status
        self.send_response(status)
        self.send_header("X-RateLimit-Resource", "core")
        used = self.quota(status)
        self.send_header("X-RateLimit-Used", str(used))
        self.send_header("X-RateLimit-Remaining", str(5000 - used))
        self.send_header("X-RateLimit-Limit", "5000")
        self.send_header("X-RateLimit-Reset", "4102444800")
        if validator:
            self.send_header("ETag", etag)
        if unchanged:
            self.end_headers()
            return
        encoded = json.dumps(payload).encode()
        if not collection:
            self.send_header("Cache-Control", "no-store")
        if next_path:
            self.send_header("Link", f'<https://api.github.com{next_path}>; rel="next"')
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def record(self, query: str | None = None) -> None:
        with self.fixture[0].open("a", encoding="utf-8") as output:
            output.write(
                json.dumps(
                    {
                        "path": self.path,
                        "query": query,
                        "accept": self.headers.get("Accept"),
                        "range": self.headers.get("Range"),
                        "validator": self.headers.get("If-None-Match"),
                    }
                )
                + "\n"
            )

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
                {
                    "total_count": len(runs),
                    "workflow_runs": rows,
                    "fixture_version": self.fixture_value("api-version", "1"),
                    "fixture_accept": self.headers.get("Accept"),
                },
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
        elif self.path == "/rate_limit":
            used = int(self.fixture_value("core-used", "0"))
            self.reply(
                {
                    "resources": {
                        "core": {"used": used, "remaining": 5000 - used, "limit": 5000}
                    }
                }
            )
        elif self.path == "/fixture/error":
            self.reply(
                {
                    "message": "Not Found",
                    "documentation_url": "https://example.invalid/rest",
                    "status": "404",
                },
                status=404,
            )
        else:
            raise AssertionError(f"undeclared API request: {self.path}")

    def do_HEAD(self) -> None:  # pylint: disable=invalid-name
        self.record()
        self.send_response(200)
        self.end_headers()

    def date_time_string(self, timestamp: float | None = None) -> str:
        del timestamp
        return "Thu, 01 Jan 2026 00:00:00 GMT"
