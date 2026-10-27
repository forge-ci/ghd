"""#16: verified offline plugin bootstrap uses the installed native publisher."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import zipfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

from ghd import __version__
from ghd.platforms import binary_suffix, cli_binary_name, is_windows

ROOT = Path(__file__).resolve().parents[2]
BOOTSTRAP = ROOT / "agent-plugin" / "bootstrap.py"


def test_pinned_plugin_install_status_uninstall(tmp_path: Path) -> None:
    binary = Path(sys.executable).parent / cli_binary_name()
    archive = tmp_path / "ghd-0.1.0-fixture.whl"
    member = f"ghd-{__version__}.data/scripts/" + cli_binary_name()
    with zipfile.ZipFile(archive, "w") as wheel:
        wheel.write(binary, member)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    destination = tmp_path / "plugin home with spaces"
    command = [
        sys.executable,
        str(BOOTSTRAP),
        "install",
        "--kind",
        "ghd",
        "--root",
        str(destination),
        "--archive",
        str(archive),
        "--member",
        member,
        "--sha256",
        checksum,
        "--version",
        __version__,
    ]
    result = subprocess.run(
        command, capture_output=True, text=True, check=False, timeout=30
    )
    assert result.returncode == 0, result.stderr
    status = subprocess.run(
        [
            sys.executable,
            str(BOOTSTRAP),
            "status",
            "--kind",
            "ghd",
            "--root",
            str(destination),
            "--ghd",
            str(binary),
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    assert status.returncode == 0, status.stderr
    record = json.loads(status.stdout)
    assert record["installed"] is True
    assert record["artifact_sha256"] == checksum
    before = (destination / "ghd.json").read_bytes()
    mismatch = command.copy()
    mismatch[mismatch.index("--sha256") + 1] = "0" * 64
    rejected = subprocess.run(
        mismatch, capture_output=True, text=True, check=False, timeout=10
    )
    assert rejected.returncode != 0 and "SHA256 mismatch" in rejected.stderr
    assert (destination / "ghd.json").read_bytes() == before
    binary_path = Path(record["path"])
    assert binary_path.is_file()
    forged = tmp_path / "public-forged-root"
    (forged / "objects").mkdir(parents=True)
    forged.chmod(0o755)
    (forged / "objects").chmod(0o755)
    (forged / "ghd.json").write_bytes(before)
    shutil.copyfile(binary_path, forged / "objects" / binary_path.name)
    (forged / "objects" / binary_path.name).chmod(0o755)
    untrusted = subprocess.run(
        [
            sys.executable,
            str(BOOTSTRAP),
            "run",
            "--root",
            str(forged),
            "--",
            "--version",
        ],
        env=dict(os.environ, GHD_PUBLISHER=str(binary)),
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    assert untrusted.returncode != 0 and "private" in untrusted.stderr
    binary_path.write_bytes(b"corrupt installed binary")
    corrupted = subprocess.run(
        [
            sys.executable,
            str(BOOTSTRAP),
            "run",
            "--root",
            str(destination),
            "--",
            "--version",
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    assert corrupted.returncode != 0 and "checksum mismatch" in corrupted.stderr
    removed = subprocess.run(
        [
            sys.executable,
            str(BOOTSTRAP),
            "uninstall",
            "--kind",
            "ghd",
            "--root",
            str(destination),
            "--ghd",
            str(binary),
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    assert removed.returncode == 0, removed.stderr
    assert not list(destination.glob("objects/ghd-*"))


def test_managed_gh_discovery_checksum_and_system_selection(tmp_path: Path) -> None:
    publisher = Path(sys.executable).parent / cli_binary_name()
    # Session launcher shims may exceed the supported release-binary limit.
    # Select a bounded system candidate without modifying or stripping it.
    candidates = [
        Path(folder) / ("gh" + binary_suffix()) for folder in os.get_exec_path()
    ]
    real_gh = next(
        (p for p in candidates if p.is_file() and p.stat().st_size <= 64 * 1024 * 1024),
        None,
    )
    if real_gh is None:
        pytest.skip(
            "bounded system GitHub CLI unavailable for verified third-party test"
        )
    version_reply = subprocess.run(
        [str(real_gh), "--version"],
        capture_output=True,
        text=True,
        check=True,
        timeout=5,
    )
    version = version_reply.stdout.split()[2]
    member = "bin/" + real_gh.name
    archive = tmp_path / "gh.zip"
    with zipfile.ZipFile(archive, "w", strict_timestamps=False) as package:
        package.write(real_gh, member)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    root = tmp_path / "managed gh"
    command = [
        sys.executable,
        str(BOOTSTRAP),
        "install",
        "--kind",
        "gh",
        "--root",
        str(root),
        "--archive",
        str(archive),
        "--member",
        member,
        "--sha256",
        checksum,
        "--version",
        version,
        "--ghd",
        str(publisher),
    ]
    with ThreadPoolExecutor(max_workers=2) as pool:
        responses = list(
            pool.map(
                lambda _: subprocess.run(
                    command, capture_output=True, text=True, check=False, timeout=30
                ),
                range(2),
            )
        )
    assert any(reply.returncode == 0 for reply in responses), [
        r.stderr for r in responses
    ]
    assert all(
        reply.returncode == 0 or "busy" in reply.stderr for reply in responses
    ), [r.stderr for r in responses]
    environment = dict(
        os.environ,
        GHD_MANAGED_GH_DIR=str(root),
        GHD_STATE_DIR=str(tmp_path / "no-daemon"),
    )
    environment.pop("GHD_GH_PATH", None)
    environment.pop("GHD_PASSTHROUGH", None)
    reply = subprocess.run(
        [str(publisher), "gh", "--version"],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    assert reply.returncode == 0 and version in reply.stdout, reply.stderr
    assert not (tmp_path / "no-daemon").exists()
    (root / "gh.json").write_text("corrupt", encoding="utf-8")
    rejected = subprocess.run(
        [str(publisher), "gh", "--version"],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    assert rejected.returncode != 0
    environment["GHD_GH_PATH"] = str(real_gh)
    selected = subprocess.run(
        [str(publisher), "gh", "--version"],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    assert selected.returncode == 0 and version in selected.stdout, selected.stderr
    shim = tmp_path / ("own-gh.cmd" if is_windows() else "own-gh")
    shim.write_text("# ghd-agent-shim\n", encoding="utf-8")
    environment["GHD_GH_PATH"] = str(shim)
    loop = subprocess.run(
        [str(publisher), "gh", "--version"],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    assert loop.returncode != 0 and "agent shim" in loop.stderr


def test_plugin_wrapper_preserves_streams_status_and_arguments(
    resource_cli, tmp_path: Path
) -> None:
    invoke, _ = resource_cli
    binary = Path(sys.executable).parent / cli_binary_name()
    archive = tmp_path / "wrapper.whl"
    member = f"ghd-{__version__}.data/scripts/" + cli_binary_name()
    with zipfile.ZipFile(archive, "w", strict_timestamps=False) as package:
        package.write(binary, member)
    root = tmp_path / "wrapper home"
    launchers = tmp_path / "trusted launchers"
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    installed = subprocess.run(
        [
            sys.executable,
            str(BOOTSTRAP),
            "install",
            "--kind",
            "ghd",
            "--root",
            str(root),
            "--archive",
            str(archive),
            "--member",
            member,
            "--sha256",
            checksum,
            "--version",
            __version__,
            "--launchers",
            str(launchers),
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=30,
    )
    assert installed.returncode == 0, installed.stderr
    wrapper = launchers / ("gh.cmd" if is_windows() else "gh")
    words = ["auth", "fixture-exit", "space value", "", "snowman-☃"]
    code = (
        "import os,subprocess,json; "
        f"r=subprocess.run({[str(wrapper), *words]!r},"
        f"env=dict(os.environ,GHD_PLUGIN_HOME={str(root)!r},GHD_PUBLISHER={str(binary)!r}),"
        "capture_output=True,text=True,check=False); "
        "print(json.dumps({'code':r.returncode,'out':r.stdout,'err':r.stderr}))"
    )
    response = invoke(python_code=code)
    assert response.returncode == 0, response.stderr
    report = json.loads(response.stdout)
    assert report["code"] == 23 and report["err"] == "fixture delegated error\n"
    assert json.loads(report["out"]) == words
    removed = subprocess.run(
        [
            sys.executable,
            str(BOOTSTRAP),
            "uninstall",
            "--root",
            str(root),
            "--kind",
            "ghd",
            "--ghd",
            str(binary),
            "--launchers",
            str(launchers),
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=15,
    )
    assert removed.returncode == 0, removed.stderr
    assert not any(
        (launchers / name).exists()
        for name in ("gh", "gh.cmd", "install.sh", "install.cmd", "bootstrap.py")
    )
    assert (launchers / ".ghd-launchers-v1").exists()


def test_cold_plugin_autostart_pins_verified_build_without_system_ghd(
    resource_cli, tmp_path: Path
) -> None:
    invoke, _ = resource_cli
    publisher = Path(sys.executable).parent / cli_binary_name()
    archive = tmp_path / "cold-plugin.whl"
    member = f"ghd-{__version__}.data/scripts/" + cli_binary_name()
    with zipfile.ZipFile(archive, "w", strict_timestamps=False) as package:
        package.write(publisher, member)
    root = tmp_path / "cold-plugin"
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    installed = subprocess.run(
        [
            sys.executable,
            str(BOOTSTRAP),
            "install",
            "--kind",
            "ghd",
            "--root",
            str(root),
            "--archive",
            str(archive),
            "--member",
            member,
            "--sha256",
            checksum,
            "--version",
            __version__,
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=30,
    )
    assert installed.returncode == 0, installed.stderr
    cold_state = tmp_path / "cold-state"
    empty_path = tmp_path / "empty-path"
    empty_path.mkdir()
    run = [
        sys.executable,
        str(BOOTSTRAP),
        "run",
        "--root",
        str(root),
        "--",
        "gh",
        "issue",
        "view",
        "42",
        "-R",
        "o/r",
        "--json",
        "title",
    ]
    stop = [str(publisher), "tools", "run", str(root), "daemon", "stop"]
    code = (
        "import subprocess,json\n"
        "try:\n"
        f" r=subprocess.run({run!r},capture_output=True,text=True,check=False,timeout=10)\n"
        "finally:\n"
        f" s=subprocess.run({stop!r},capture_output=True,text=True,check=False,timeout=15)\n"
        "print(json.dumps({'code':r.returncode,'out':r.stdout,'err':r.stderr,'stop':s.returncode,'stop_err':s.stderr}))\n"
    )
    response = invoke(
        python_code=code,
        timeout=30,
        context={
            "GHD_STATE_DIR": str(cold_state),
            "GHD_EXE": None,
            "GHD_PUBLISHER": str(publisher),
            "PATH": str(empty_path),
        },
    )
    assert response.returncode == 0, response.stderr
    result = json.loads(response.stdout)
    assert result["code"] == 0, result["err"]
    assert result["stop"] == 0, result["stop_err"]
    assert "title" in json.loads(result["out"])
    assert not list(cold_state.glob("identities-v1/*/endpoint"))
