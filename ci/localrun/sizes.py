"""Byte-size parsing, matching `ci_lint`'s own convention exactly.

Duplicated (not imported) from `zackees/ci.yml`'s
`ci_lint/rules/cache_static.py::parse_size` -- binary units (1 KB =
1024 B), the same regex. Kept here as a five-line stdlib helper rather
than an import so `ci/local.py act-inner` (which runs inside the bosn
`act` stack container, with no `ci_lint` checkout mounted at a
predictable path in every invocation shape) never depends on `ci_lint`
being importable.
"""

from __future__ import annotations

import re

_SIZE_RE = re.compile(r"^(\d+(?:\.\d+)?)\s*(B|KB|MB|GB)$", re.IGNORECASE)
_SIZE_UNITS: dict[str, int] = {"B": 1, "KB": 1024, "MB": 1024**2, "GB": 1024**3}


def parse_size(text: str) -> int | None:
    m = _SIZE_RE.match(text.strip())
    if not m:
        return None
    value = float(m.group(1))
    unit = m.group(2).upper()
    return int(value * _SIZE_UNITS[unit])


def format_size(num_bytes: int) -> str:
    value = float(num_bytes)
    for unit in ("B", "KB", "MB", "GB"):
        if value < 1024.0 or unit == "GB":
            return f"{value:.1f}{unit}" if unit != "B" else f"{int(value)}{unit}"
        value /= 1024.0
    return f"{value:.1f}GB"
