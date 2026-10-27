"""#14: installed operational controls preserve shared cache custody."""

from __future__ import annotations

import http.server
import json
import subprocess
import threading
import time
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ResourceCli = tuple[Callable[..., subprocess.CompletedProcess[str]], Path]


def test_cache_flush_and_redacted_inspection(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    args = ("issue", "view", "42", "-R", "o/r", "--json", "title")
    first = invoke(*args)
    assert first.returncode == 0, first.stderr
    inspected = invoke("cache", "keys", native=True)
    assert inspected.returncode == 0, inspected.stderr
    keys = json.loads(inspected.stdout)
    assert keys["keys"]
    assert "resource-fixture-token" not in inspected.stdout
    assert "snapshot title" not in inspected.stdout
    flushed = invoke("cache", "flush", native=True)
    assert flushed.returncode == 0, flushed.stderr
    second = invoke(*args)
    assert second.returncode == 0, second.stderr
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 2


def test_per_request_bypass_does_not_reconfigure_other_callers(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    args = ("gh", "issue", "view", "42", "-R", "o/r", "--json", "title")
    for prefix in [(), ("--no-cache",), (), ("--ttl", "0"), ()]:
        reply = invoke(*prefix, *args, native=True)
        assert reply.returncode == 0, reply.stderr
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 3


def test_lifecycle_serializes_restart_and_never_uses_pid_signals(
    resource_cli: ResourceCli,
) -> None:
    invoke, _ = resource_cli
    healthy = invoke("daemon", "status", native=True)
    assert healthy.returncode == 0 and json.loads(healthy.stdout)["status"] == "healthy"
    stopped = invoke("daemon", "stop", native=True)
    assert stopped.returncode == 0, stopped.stderr
    missing = invoke("daemon", "status", native=True)
    assert missing.returncode == 3 and json.loads(missing.stdout)["status"] == "missing"
    try:
        with ThreadPoolExecutor(max_workers=4) as pool:
            results = list(
                pool.map(lambda _: invoke("daemon", "start", native=True), range(4))
            )
        assert all(result.returncode == 0 for result in results), [
            r.stderr for r in results
        ]
        restarted = invoke("daemon", "restart", native=True)
        assert restarted.returncode == 0, restarted.stderr
        assert invoke("daemon", "status", native=True).returncode == 0
    finally:
        final = invoke("daemon", "stop", native=True)
        assert final.returncode == 0, final.stderr


def test_binding_cache_controls_share_the_cli_cache(resource_cli: ResourceCli) -> None:
    invoke, trace = resource_cli
    code = (
        "from ghd.bindings import gh_read; "
        "args=['issue','view','42','-R','o/r','--json','title']; "
        "assert gh_read(args).exit_code == 0; "
        "assert gh_read(args,no_cache=True).exit_code == 0; "
        "assert gh_read(args).source == 'fresh'; "
        "assert gh_read(args,ttl_ms=0).exit_code == 0"
    )
    response = invoke(python_code=code)
    assert response.returncode == 0, response.stderr
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 3


def test_configuration_precedence_and_invalid_restart_preserve_custody(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    config = next((trace.parent / "state").glob("identities-v1/*")) / "ghd.json"
    config.write_text('{"GHD_FRESH_MS":0}', encoding="utf-8")
    config.chmod(0o600)
    args = ("issue", "view", "42", "-R", "o/r", "--json", "title")
    try:
        started = invoke(
            "daemon",
            "restart",
            native=True,
            context={"GHD_CONFIG": str(config), "GHD_FRESH_MS": None},
        )
        assert started.returncode == 0, started.stderr
        for _ in range(2):
            response = invoke(*args)
            assert response.returncode == 0, response.stderr
        assert len(trace.read_text(encoding="utf-8").splitlines()) == 2
        overridden = invoke(
            "daemon", "restart", native=True, context={"GHD_CONFIG": str(config)}
        )
        assert overridden.returncode == 0, overridden.stderr
        for _ in range(2):
            response = invoke(*args)
            assert response.returncode == 0, response.stderr
        assert len(trace.read_text(encoding="utf-8").splitlines()) == 3
        config.write_text('{"unknown":0}', encoding="utf-8")
        rejected = invoke(
            "daemon", "restart", native=True, context={"GHD_CONFIG": str(config)}
        )
        assert rejected.returncode != 0 and "invalid GHD_CONFIG" in rejected.stderr
        assert invoke("daemon", "status", native=True).returncode == 0
    finally:
        assert invoke("daemon", "stop", native=True).returncode == 0


def test_stop_refuses_active_work_and_flush_fences_old_publishers(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    hold = trace.with_name("operation-hold")
    hold.write_text("held", encoding="utf-8")
    args = ("issue", "view", "42", "-R", "o/r", "--json", "title")
    with ThreadPoolExecutor(max_workers=1) as pool:
        pending = pool.submit(invoke, *args)
        try:
            deadline = time.monotonic() + 5
            while not trace.with_name("operation-started").exists():
                assert time.monotonic() < deadline, "upstream did not start"
                time.sleep(0.01)
            stopped = invoke("daemon", "stop", native=True)
            assert stopped.returncode != 0 and "active" in stopped.stderr
            flushed = invoke("cache", "flush", native=True)
            assert flushed.returncode == 0, flushed.stderr
        finally:
            hold.write_text("released", encoding="utf-8")
        result = pending.result()
    assert result.returncode != 0
    assert not result.stdout
    retry = invoke(*args)
    assert retry.returncode == 0, retry.stderr
    assert len(trace.read_text(encoding="utf-8").splitlines()) == 2


def test_controlled_snapshot_refuses_an_old_daemon_that_ignores_policy(
    resource_cli: ResourceCli,
) -> None:
    invoke, trace = resource_cli
    endpoint = next((trace.parent / "state").glob("identities-v1/*/endpoint"))
    original = endpoint.read_bytes()
    paths: list[str] = []

    class OldSnapshotDaemon(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args: object, **_kwargs: object) -> None:
            pass

        def do_POST(self) -> None:  # pylint: disable=invalid-name
            paths.append(self.path)
            self.rfile.read(int(self.headers.get("Content-Length", "0")))
            # The #13 route knows no request-policy header and returns cached JSON.
            body = b'\x0a\x0f{"cached":true}\x12\x05fresh'
            self.send_response(200 if self.path == "/v2/snapshot" else 404)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    with http.server.HTTPServer(("127.0.0.1", 0), OldSnapshotDaemon) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            token = original.decode().split()[1]
            endpoint.write_text(f"{server.server_port} {token}\n", encoding="utf-8")
            ordinary = invoke("snapshot", "-R", "o/r", native=True)
            assert ordinary.returncode == 0, ordinary.stderr
            for prefix in [("--no-cache",), ("--ttl", "0")]:
                refused = invoke(*prefix, "snapshot", "-R", "o/r", native=True)
                assert refused.returncode != 0 and not refused.stdout
                assert "unsupported" in refused.stderr
            assert paths == ["/v2/snapshot", "/v3/snapshot", "/v3/snapshot"]
        finally:
            endpoint.write_bytes(original)
            server.shutdown()
            thread.join(timeout=5)
