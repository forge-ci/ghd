"""Read just the `[cache]` and `[local]` tables of `ci.toml` with `tomllib`.

`ci_lint.schema` is the authoritative, strictly-validated parser, but it
only ever runs where the pinned `ci_lint` checkout is on `PYTHONPATH`
(host, after `ensure_ci_lint`). `ci/local.py act-inner` runs inside the
bosn `act` stack container, which never clones `ci_lint` (it only needs
two scalar-ish fields, not a full contract check -- precheck already
strictly validated the whole file before act-inner ever starts). This
module is a narrow, stdlib-only reader for exactly those fields, so
act-inner has no dependency on where `.ci-lint` lives.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

import tomllib


class CiTomlLiteError(RuntimeError):
    pass


@dataclass(frozen=True)
class CacheSection:
    budget: str
    retired: tuple[str, ...]


@dataclass(frozen=True)
class LocalSection:
    runner: str
    lanes: tuple[str, ...]
    cache: str


def _load(repo_root: Path) -> dict[str, object]:
    path = repo_root / "ci.toml"
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise CiTomlLiteError(f"cannot read/parse {path}: {exc}") from exc


def read_cache_section(repo_root: Path) -> CacheSection:
    doc = _load(repo_root)
    cache = doc.get("cache")
    if not isinstance(cache, dict):
        raise CiTomlLiteError("ci.toml has no [cache] table")
    budget = cache.get("budget")
    retired = cache.get("retired", [])
    if not isinstance(budget, str):
        raise CiTomlLiteError("ci.toml [cache].budget must be a string")
    if not isinstance(retired, list) or not all(isinstance(r, str) for r in retired):
        raise CiTomlLiteError("ci.toml [cache].retired must be a list of strings")
    return CacheSection(budget=budget, retired=tuple(retired))


def read_local_section(repo_root: Path) -> LocalSection:
    doc = _load(repo_root)
    local = doc.get("local")
    if not isinstance(local, dict):
        raise CiTomlLiteError("ci.toml has no [local] table")
    runner = local.get("runner")
    lanes = local.get("lanes", [])
    cache = local.get("cache")
    if not isinstance(runner, str) or not isinstance(cache, str):
        raise CiTomlLiteError(
            "ci.toml [local].runner and [local].cache must be strings"
        )
    if not isinstance(lanes, list) or not all(isinstance(item, str) for item in lanes):
        raise CiTomlLiteError("ci.toml [local].lanes must be a list of strings")
    return LocalSection(runner=runner, lanes=tuple(lanes), cache=cache)
