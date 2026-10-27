"""Thin Python wrappers around the PyO3 extension module."""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from ghd import _native


def version_banner() -> str:
    """Return ``ghd <version> (<os>)`` from the native extension."""
    return _native.version_banner()


def pr_view(repo: str, number: int, fields: list[str]) -> dict[str, Any]:
    """``gh pr view <number> -R <repo> --json <fields>`` through the ghd daemon.

    Starts the daemon (the bundled ``ghd`` binary) on first use.
    """
    owner, name = repo.rsplit("/", 1)
    return json.loads(_native.pr_view(owner, name, number, fields))


@dataclass(frozen=True)
class ReadResult:
    """Native read result; reaching a list limit does not prove completeness."""

    stdout: bytes
    stderr: bytes
    exit_code: int
    source: str
    limit_reached: bool


def gh_read(args: list[str]) -> ReadResult:
    """Run a safe, explicitly bound read using the same cache as the CLI.

    The result preserves pending-checks exit 8 and raw output bytes. Transport
    and identity failures raise OSError. Writes and interactive forms belong
    to the CLI compatibility route and are rejected by this read-only API.
    """
    # PyO3 returns the tuple declared in _native.pyi; astroid cannot infer it.
    return ReadResult(*_native.gh_read(args))  # pylint: disable=not-an-iterable
