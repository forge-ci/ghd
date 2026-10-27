"""Archive integrity rejection does not require network or executing artifacts."""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import importlib.util
import io
import tarfile
import zipfile
from pathlib import Path
from unittest.mock import Mock

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "agent-plugin" / "bootstrap.py"


@pytest.fixture(name="bootstrap")
def bootstrap_module():
    spec = importlib.util.spec_from_file_location("ghd_plugin_bootstrap", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_checksum_mismatch_and_insecure_download_are_rejected(
    bootstrap, tmp_path: Path
) -> None:
    archive = tmp_path / "artifact"
    archive.write_bytes(b"corrupt download")
    args = argparse.Namespace(archive=str(archive), url=None, sha256="0" * 64)
    with pytest.raises(ValueError, match="SHA256 mismatch"):
        bootstrap.artifact(args)
    args.archive = None
    args.url = "http://127.0.0.1/never-contact"
    with pytest.raises(ValueError, match="HTTPS"):
        bootstrap.artifact(args)


def test_exact_regular_zip_member_and_traversal_rejection(
    bootstrap, tmp_path: Path
) -> None:
    archive = tmp_path / "tool.zip"
    with zipfile.ZipFile(archive, "w") as package:
        package.writestr("bin/gh", b"verified body")
    assert bootstrap.extract(archive, "bin/gh", "gh") == b"verified body"
    for member in ("../bin/gh", "/bin/gh", "bin/not-gh"):
        with pytest.raises(ValueError):
            bootstrap.extract(archive, member, "gh")
    for mode in (0o120777, 0o010600, 0o020600, 0o040700):
        link = zipfile.ZipInfo("bin/gh")
        link.external_attr = mode << 16
        with zipfile.ZipFile(archive, "w") as package:
            package.writestr(link, "other-path")
        with pytest.raises(ValueError, match="regular"):
            bootstrap.extract(archive, "bin/gh", "gh")


def test_tar_link_is_not_materialized_and_sha_pin_precedes_use(
    bootstrap, tmp_path: Path
) -> None:
    archive = tmp_path / "tool.tar.gz"
    with tarfile.open(archive, "w:gz") as package:
        regular = tarfile.TarInfo("bin/gh")
        regular.size = 4
        package.addfile(regular, io.BytesIO(b"data"))
    args = argparse.Namespace(
        archive=str(archive),
        url=None,
        sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),
    )
    assert bootstrap.artifact(args) == archive.read_bytes()
    assert bootstrap.extract(archive, "bin/gh", "gh") == b"data"
    with tarfile.open(archive, "w:gz") as package:
        link = tarfile.TarInfo("bin/gh")
        link.type = tarfile.SYMTYPE
        link.linkname = "/external"
        package.addfile(link)
    with pytest.raises(ValueError, match="regular"):
        bootstrap.extract(archive, "bin/gh", "gh")


def test_bounded_read_and_downgrade_redirect_are_rejected(bootstrap) -> None:
    with pytest.raises(ValueError, match="byte limit"):
        bootstrap.bounded(io.BytesIO(b"over-limit"), 3)
    redirect = bootstrap.SecureRedirect()
    with pytest.raises(ValueError, match="HTTPS"):
        redirect.redirect_request(
            None, None, 302, "redirect", {}, "http://never-contact.invalid/tool"
        )


def test_publisher_output_is_bounded_while_collecting(bootstrap) -> None:
    async def check() -> None:
        done = asyncio.get_running_loop().create_future()
        protocol = bootstrap.ProbeOutput(done)
        transport = Mock(spec=asyncio.SubprocessTransport)
        protocol.connection_made(transport)
        protocol.pipe_data_received(1, b"x" * 1000000)
        assert len(protocol.output) == 65537
        transport.close.assert_called_once()
        with pytest.raises(ValueError, match="byte limit"):
            await done

    asyncio.run(check())
