"""The `integration` suite (`ci.toml [suites].integration`): the installed
package and the native CLI, exercised together, end to end.

Every test in `tests/` (unit suite) proves the extension and the CLI *each*
work; these prove they come from the same build and agree with each other:

  1. `ghd.bindings.version_banner()` (the PyO3 extension) and `ghd
     --version` (the bundled native binary, `crates/ghd-cli/src/ghd.rs`)
     print the same `ghd <version> (<os>)` banner, byte for byte. Both call
     the amalgam's one `version_banner()`; a stale cache or a wrong target
     bundled into one surface but not the other would pass every unit test
     and fail here.
  2. The binary rejects a malformed `ghd pr view` before any network or
     daemon call, so argument handling is proven without GitHub access.

Network-backed reads (`ghd pr view` against a real repository) are not
here: CI has no GitHub token budget to spend on them, and the daemon's
cache/batching logic is unit-tested in `ghd-daemon`.

Marked `integration`, so the default `pytest` run excludes it; it runs
when `ci.toml`'s `integration` suite is selected (`[ci-full]`). Assumes
the package is installed and `ghd` is on PATH, exactly what the fast
lane's `uv sync` and every platform-run lane's clean-venv wheel install
guarantee before this file runs.
"""

from __future__ import annotations

import shutil
import subprocess

import pytest

from ghd import bindings
from ghd.platforms import cli_binary_name, os_name

pytestmark = pytest.mark.integration


def _cli_path() -> str:
    binary = shutil.which(cli_binary_name())
    assert binary, (
        f"{cli_binary_name()} not on PATH -- the integration suite requires the package to already be "
        "installed (fast lane: 'uv sync --frozen'; platform-run lanes: a clean-venv wheel install)"
    )
    return binary


def _run_cli(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [_cli_path(), *args], capture_output=True, text=True, timeout=30, check=False
    )


def test_python_binding_and_native_cli_print_the_same_banner() -> None:
    proc = _run_cli("--version")
    assert proc.returncode == 0, f"ghd --version failed: {proc.stderr}"
    assert proc.stdout.strip() == bindings.version_banner()


def test_banner_names_this_host() -> None:
    assert f"({os_name()})" in str(bindings.version_banner())


def test_native_cli_rejects_a_malformed_pr_view_without_the_network() -> None:
    proc = _run_cli("pr", "view", "-R", "o/r", "--json", "state")
    assert proc.returncode != 0
    assert "missing PR number" in proc.stderr


def test_native_cli_help_lists_every_subcommand() -> None:
    proc = _run_cli("--help")
    assert proc.returncode == 0
    for sub in ("pr view", "stats", "daemon"):
        assert sub in proc.stdout
