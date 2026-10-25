"""Sanity check: `ghd` is on PATH after `pip install`.

The wheel ships `ghd[.exe]` as a raw script at
`<name>-<ver>.data/scripts/`, bundled in by the soldr PEP 517 backend's
`[tool.soldr.pep517] bundle-bins` (see `pyproject.toml` and
zackees/ci.yml#6 round 1, which replaced the earlier post-build
zipfile-injection step, `ci/build_wheel.py`). pip drops that script
straight into the venv's `Scripts/` (Windows) or `bin/` (POSIX)
directory. There is no Python shim in front of it — see #7 for why
`os.execv`-based launchers race the shell prompt on Windows.

This test is a *runtime contract* check: if a downstream user does
`pip install ghd`, can they then run
`ghd --version`? It probes PATH directly rather than the
package source tree, because the wheel-side delivery mechanism (raw
script) is fundamentally not visible from the source tree.

Per zackees/ci.yml#6 round 1, this test does NOT skip when the binary
is missing — it fails. `./test` (`ci/gates/test.py`) always installs
the project through the soldr backend before running pytest, so the
binary is expected to be on PATH by the time this file runs; a missing
binary means the packaging path broke, which is exactly what this test
exists to catch.
"""

from __future__ import annotations

import os
import shutil
import subprocess

from ghd.platforms import cli_binary_name


def _cli_on_path() -> str:
    binary = shutil.which("ghd")
    assert binary is not None, (
        "ghd not on PATH. This test runs after the project is "
        "installed through the soldr PEP 517 backend (`uv sync`, which "
        "`./test` / `ci/gates/test.py` always does first) so "
        "`[tool.soldr.pep517] bundle-bins` has staged the binary into "
        "the venv's Scripts/bin directory. Run `./test`, or `uv sync` "
        "followed by `uv run pytest`, to exercise this gate locally."
    )
    return binary


def test_cli_on_path_has_expected_name() -> None:
    binary = _cli_on_path()
    assert os.path.basename(binary).lower() == cli_binary_name().lower()


def test_cli_on_path_invokes() -> None:
    """The PATH-resolved `ghd` runs and exits with code 0 on --version.

    Doubles as a Windows stdout-ordering smoke test: if the wheel
    accidentally regressed back to a Python launcher (e.g. someone
    added `[project.scripts]` again), `subprocess.run` would still
    succeed but the test wouldn't catch the cmd.exe shell-prompt race
    — that one needs an interactive console. The `file` / `unzip -l`
    check in #2's acceptance criteria covers the static shape; this
    test covers "the binary runs at all".
    """
    binary = _cli_on_path()
    proc = subprocess.run(
        [binary, "--version"],
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout.strip(), "ghd --version produced empty stdout"
