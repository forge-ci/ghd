"""Thin Python wrappers around the PyO3 extension module."""

from __future__ import annotations

import json
from typing import Any

from ghd import _native


def version_banner() -> str:
    """Return ``ghd <version> (<os>)`` from the native extension."""
    return _native.version_banner()


def pr_view(repo: str, number: int, fields: list[str]) -> dict[str, Any]:
    """``gh pr view <number> -R <repo> --json <fields>`` through the ghd daemon.

    Starts the daemon (the bundled ``ghd`` binary) on first use.
    """
    owner, name = repo.split("/", 1)
    return json.loads(_native.pr_view(owner, name, number, fields))
