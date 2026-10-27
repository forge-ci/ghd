#!/usr/bin/env python3
# ghd-agent-shim: trusted launcher payload, never a real GitHub CLI.
"""Explicit checksum-pinned native tool bootstrap; no network on normal calls."""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import os
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from email.message import Message
from pathlib import Path, PurePosixPath
from typing import BinaryIO, cast
from urllib.parse import urlsplit

MAX_ARCHIVE = 128 * 1024 * 1024
MAX_BINARY = 64 * 1024 * 1024


class SecureRedirect(urllib.request.HTTPRedirectHandler):
    # The standard-library redirect hook requires all six positional arguments.
    # pylint: disable=too-many-arguments,too-many-positional-arguments
    def redirect_request(
        self,
        req: urllib.request.Request,
        fp: BinaryIO,
        code: int,
        msg: str,
        headers: Message,
        newurl: str,
    ) -> urllib.request.Request | None:
        parsed = urlsplit(newurl)
        if parsed.scheme != "https" or parsed.username or parsed.password:
            raise ValueError("artifact redirect left credential-free HTTPS")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def bounded(stream: BinaryIO, limit: int) -> bytes:
    data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("artifact exceeds bootstrap byte limit")
    return data


def artifact(args: argparse.Namespace) -> bytes:
    if args.archive:
        with Path(args.archive).open("rb") as stream:
            data = bounded(stream, MAX_ARCHIVE)
    else:
        parsed = urlsplit(args.url)
        if (
            parsed.scheme != "https"
            or not parsed.hostname
            or any((parsed.username, parsed.password, parsed.query, parsed.fragment))
        ):
            raise ValueError(
                "download requires an explicit credential-free HTTPS artifact URL"
            )
        with urllib.request.build_opener(SecureRedirect()).open(
            args.url, timeout=60
        ) as stream:
            if urlsplit(stream.geturl()).scheme != "https":
                raise ValueError("artifact redirect left HTTPS")
            data = bounded(stream, MAX_ARCHIVE)
    if hashlib.sha256(data).hexdigest() != args.sha256.lower():
        raise ValueError("artifact SHA256 mismatch; no publication performed")
    return data


def extract(archive: Path, member: str, kind: str) -> bytes:
    parts = PurePosixPath(member)
    if (
        parts.is_absolute()
        or ".." in parts.parts
        or parts.name not in (kind, kind + ".exe")
    ):
        raise ValueError("select the exact regular tool member")
    if zipfile.is_zipfile(archive):
        return zip_member(archive, member)
    return tar_member(archive, member)


def zip_member(archive: Path, member: str) -> bytes:
    with zipfile.ZipFile(archive) as package:
        matches = [info for info in package.infolist() if info.filename == member]
        if len(matches) != 1 or len(package.infolist()) > 4096:
            raise ValueError("missing/duplicate member or archive entry limit")
        info = matches[0]
        mode = (info.external_attr >> 16) & 0o170000
        if (
            mode not in (0, 0o100000)
            or info.is_dir()
            or info.external_attr & 0x10
            or info.flag_bits & 1
            or info.file_size > MAX_BINARY
        ):
            raise ValueError("tool member is not a bounded regular file")
        with package.open(info) as stream:
            return bounded(stream, MAX_BINARY)


def tar_member(archive: Path, member: str) -> bytes:
    with tarfile.open(archive, "r:*") as package:
        selected = None
        for count, info in enumerate(package):
            if count >= 4096:
                raise ValueError("archive entry limit")
            if info.name == member:
                if selected is not None or not info.isfile() or info.size > MAX_BINARY:
                    raise ValueError("tool member must be unique, bounded and regular")
                stream = package.extractfile(info)
                if stream is None:
                    raise ValueError("missing tool data")
                with stream:
                    selected = bounded(stream, MAX_BINARY)
        if selected is None:
            raise ValueError("tool member missing")
        return selected


class ProbeOutput(asyncio.SubprocessProtocol):
    """Own a bounded help buffer and close the process transport on refusal."""

    def __init__(self, done: asyncio.Future[tuple[int, bytes]]) -> None:
        self.done = done
        self.closed = asyncio.get_running_loop().create_future()
        self.output = bytearray()
        self.transport: asyncio.SubprocessTransport | None = None
        self.exited = False
        self.eof = False

    def connection_made(self, transport: asyncio.BaseTransport) -> None:
        self.transport = cast(asyncio.SubprocessTransport, transport)

    def pipe_data_received(self, fd: int, data: bytes) -> None:
        if fd != 1:
            return
        self.output.extend(data[: max(0, 65537 - len(self.output))])
        if len(self.output) > 65536 and not self.done.done():
            self.done.set_exception(ValueError("publisher help exceeds byte limit"))
            if self.transport is not None:
                self.transport.close()

    def pipe_connection_lost(self, fd: int, exc: Exception | None) -> None:
        if fd != 1:
            return
        self.eof = True
        if exc is not None and not self.done.done():
            self.done.set_exception(exc)
        self.complete()

    def connection_lost(self, exc: Exception | None) -> None:
        if exc is not None and not self.done.done():
            self.done.set_exception(exc)
        if not self.closed.done():
            self.closed.set_result(None)

    def process_exited(self) -> None:
        self.exited = True
        self.complete()

    def complete(self) -> None:
        if (
            self.eof
            and self.exited
            and not self.done.done()
            and self.transport is not None
        ):
            status = self.transport.get_returncode()
            if status is None:
                self.done.set_exception(ValueError("publisher exit status unavailable"))
            else:
                self.done.set_result((status, bytes(self.output)))


async def publisher_output(publisher: Path) -> tuple[int, bytes]:
    loop = asyncio.get_running_loop()
    done = loop.create_future()
    transport, protocol = await loop.subprocess_exec(
        lambda: ProbeOutput(done),
        str(publisher),
        "--help",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    try:
        return await asyncio.wait_for(done, timeout=5)
    finally:
        transport.close()
        await asyncio.wait_for(asyncio.shield(protocol.closed), timeout=5)


def publisher_ready(publisher: Path) -> None:
    status, output = asyncio.run(publisher_output(publisher))
    if status != 0 or b"ghd tools publish" not in output:
        raise ValueError(
            "publisher lacks verified-tool support; select a build containing the plugin feature"
        )


def checked(command: list[str]) -> None:
    subprocess.run(command, check=True, timeout=30)


def launcher_templates(directory: Path, root: Path) -> None:
    script = Path(__file__).resolve()
    if script.is_relative_to(root.resolve()):
        raise ValueError(
            "bootstrap source must be trusted separately outside managed root"
        )
    (directory / "bootstrap.py").write_bytes(script.read_bytes())
    python = shlex.quote(sys.executable)
    shell_prefix = '#!/bin/sh\n# ghd-agent-shim\nplugin_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\n'
    cmd_prefix = "@echo off\nREM ghd-agent-shim\n"
    (directory / "gh").write_text(
        shell_prefix + f'exec {python} "$plugin_dir/bootstrap.py" run '
        '--root "${GHD_PLUGIN_HOME:?set verified root}" -- gh "$@"\n',
        encoding="utf-8",
    )
    (directory / "gh.cmd").write_text(
        cmd_prefix
        + 'python "%~dp0bootstrap.py" run --root "%GHD_PLUGIN_HOME%" -- gh %*\nexit /b %errorlevel%\n',
        encoding="utf-8",
    )
    (directory / "install.sh").write_text(
        shell_prefix + f'exec {python} "$plugin_dir/bootstrap.py" "$@"\n',
        encoding="utf-8",
    )
    (directory / "install.cmd").write_text(
        cmd_prefix + 'python "%~dp0bootstrap.py" %*\nexit /b %errorlevel%\n',
        encoding="utf-8",
    )


def install(args: argparse.Namespace) -> None:
    if args.launchers:
        if (
            args.kind != "ghd"
            or not args.launchers.is_absolute()
            or not args.launchers.parent.is_dir()
        ):
            raise ValueError(
                "ghd launchers need an absolute directory with an existing parent"
            )
        if args.launchers.resolve().is_relative_to(args.root.resolve()):
            raise ValueError(
                "launchers must be trusted separately outside managed root"
            )
    data = artifact(args)
    with tempfile.TemporaryDirectory(prefix="ghd-bootstrap-") as temporary:
        stage = Path(temporary)
        archive = stage / "artifact"
        archive.write_bytes(data)
        body = extract(archive, args.member, args.kind)
        source = stage / PurePosixPath(args.member).name
        source.write_bytes(body)
        source.chmod(0o700)
        publisher = source if args.kind == "ghd" else Path(args.ghd or "ghd")
        publisher_ready(publisher)
        checked(
            [
                str(publisher),
                "tools",
                "publish",
                str(args.root),
                args.kind,
                str(source),
                args.version,
                args.sha256.lower(),
                hashlib.sha256(body).hexdigest(),
            ]
        )

        if args.launchers:
            if args.launchers.resolve().is_relative_to(args.root.resolve()):
                raise ValueError(
                    "launchers must be trusted separately outside managed root"
                )
            templates = stage / "launchers"
            templates.mkdir()
            launcher_templates(templates, args.root)
            checked(
                [
                    str(publisher),
                    "tools",
                    "launchers",
                    str(args.root),
                    str(args.launchers),
                    str(templates),
                ]
            )


def runtime_publisher(args: argparse.Namespace) -> Path:
    selected = args.ghd or os.environ.get("GHD_PUBLISHER") or shutil.which("ghd")
    if not selected:
        raise ValueError(
            "runtime needs a separately trusted publisher on PATH or GHD_PUBLISHER"
        )
    publisher = Path(shutil.which(str(selected)) or selected).resolve()
    if publisher.is_relative_to(args.root.resolve()):
        raise ValueError(
            "runtime publisher must be trusted separately, outside the installation root"
        )
    publisher_ready(publisher)
    return publisher


def lifecycle(args: argparse.Namespace) -> int:
    publisher = runtime_publisher(args)
    launchers = getattr(args, "launchers", None)
    if launchers:
        if args.command != "uninstall" or args.kind != "ghd":
            raise ValueError("launcher removal belongs to ghd uninstall")
        checked([str(publisher), "tools", "remove-launchers", str(launchers)])
    return subprocess.run(
        [str(publisher), "tools", args.command, str(args.root), args.kind],
        check=False,
        timeout=30,
    ).returncode


def run(args: argparse.Namespace) -> int:
    publisher = runtime_publisher(args)
    words = args.words[1:] if args.words[:1] == ["--"] else args.words
    result = subprocess.run(
        [str(publisher), "tools", "run", str(args.root), *words], check=False
    )
    return 128 - result.returncode if result.returncode < 0 else result.returncode


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("install", "upgrade", "status", "uninstall", "run"):
        command = commands.add_parser(name)
        command.add_argument("--root", type=Path, required=True)
        command.add_argument("--ghd")
        if name == "run":
            command.add_argument("words", nargs=argparse.REMAINDER)
            continue
        command.add_argument("--kind", choices=("gh", "ghd"), required=True)
        if name == "uninstall":
            command.add_argument("--launchers", type=Path)
        if name in ("install", "upgrade"):
            source = command.add_mutually_exclusive_group(required=True)
            source.add_argument("--archive")
            source.add_argument("--url")
            command.add_argument("--sha256", required=True)
            command.add_argument("--version", required=True)
            command.add_argument("--member", required=True)
            command.add_argument("--launchers", type=Path)
    args = parser.parse_args(argv)
    if not args.root.is_absolute():
        parser.error("--root must be absolute")
    try:
        if args.command in ("install", "upgrade"):
            if len(args.sha256) != 64 or any(
                char not in "0123456789abcdefABCDEF" for char in args.sha256
            ):
                raise ValueError("--sha256 needs 64 hexadecimal characters")
            install(args)
            return 0
        if args.command == "run":
            return run(args)
        return lifecycle(args)
    except (
        asyncio.TimeoutError,
        OSError,
        ValueError,
        KeyError,
        subprocess.SubprocessError,
        tarfile.TarError,
        zipfile.BadZipFile,
    ) as error:
        print(f"ghd bootstrap: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
