"""ACT-001: audit act's local cache-server store against remote cache policy.

zackees/ci.yml#6 section 11, rule 4: "Local caches obey remote policy.
Under act, the cache audit reads the local cache store instead of the
REST API. The same budget, `retired` list and `never` list apply, and a
saved retired key fails the run (ACT-001)." This is the zccache#1760
fixture, reproduced against a local act cache instead of the live
GitHub Actions cache API -- `ci_lint`'s own `ACT-001` is still a stub
(round 4, `ci_lint/runtime_stub.py`), so this module is the only place
that rule runs today.

act's built-in Actions-cache-server (`--cache-server-path`) persists a
bbolt (Go embedded key/value store) index at `<path>/bolt.db`, plus raw
payload blobs under `<path>/cache/**`. There is no documented on-disk
schema (zackees/ci.yml#6 open question 10) and no pure-stdlib bbolt
reader, but bbolt never compresses or encrypts a value: every index
record round-trips through the store as one literal JSON object,
recoverable with a direct byte-level regex scan of `bolt.db`. This was
verified empirically (see "act's cache-server on-disk format" in
`ci/localrun/README.md`) against a real `actions/cache/save` run under
act 0.2.88. Because bbolt is a
copy-on-write B+tree, stale pre-write copies of a record can still be
present in the file; entries are deduplicated by `id`, preferring any
sighting with `"complete":true` (an entry only ever transitions
incomplete -> complete, never back) and otherwise the most recently
`usedAt` copy.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path

from ci.localrun.sizes import format_size

_RECORD_RE = re.compile(
    rb'\{"id":\d+,"key":"[^"]*","version":"[^"]*","cacheSize":\d+,'
    rb'"complete":(?:true|false),"usedAt":\d+,"createdAt":\d+\}'
)

RULE = "ACT-001"


@dataclass(frozen=True)
class CacheIndexEntry:
    id: int
    key: str
    cache_size: int
    complete: bool
    used_at: int


@dataclass(frozen=True)
class ActCacheAuditResult:
    store_path: Path
    disk_bytes: int  # ground truth: real bytes on disk under cache/
    budget_bytes: int
    entries: tuple[CacheIndexEntry, ...]  # deduplicated, complete-only
    retired_hits: tuple[
        tuple[CacheIndexEntry, str], ...
    ]  # (entry, matched retired family)
    findings: tuple[str, ...] = field(default_factory=tuple)

    @property
    def ok(self) -> bool:
        return not self.findings


def _read_index(bolt_db: Path) -> list[CacheIndexEntry]:
    if not bolt_db.is_file():
        return []
    data = bolt_db.read_bytes()
    by_id: dict[int, dict[str, object]] = {}
    for m in _RECORD_RE.finditer(data):
        try:
            obj = json.loads(m.group(0))
        except json.JSONDecodeError:
            continue
        rid = obj["id"]
        prev = by_id.get(rid)
        if prev is None:
            by_id[rid] = obj
        elif obj["complete"] and not prev["complete"]:
            by_id[rid] = obj
        elif obj["complete"] == prev["complete"] and obj["usedAt"] >= prev["usedAt"]:
            by_id[rid] = obj
    return [
        CacheIndexEntry(
            id=obj["id"],
            key=obj["key"],
            cache_size=obj["cacheSize"],
            complete=obj["complete"],
            used_at=obj["usedAt"],
        )
        for obj in by_id.values()
        if obj["complete"]
    ]


def _disk_bytes(cache_dir: Path) -> int:
    total = 0
    blobs = cache_dir / "cache"
    if not blobs.is_dir():
        return 0
    for path in blobs.rglob("*"):
        if path.is_file():
            try:
                total += path.stat().st_size
            except OSError:
                continue
    return total


def audit(
    store_path: Path, *, budget_bytes: int, retired_families: tuple[str, ...]
) -> ActCacheAuditResult:
    """Audit `store_path` (an act `--cache-server-path` directory).

    Two independent checks, both from `ci.toml [cache]`:
      1. total on-disk bytes <= budget
      2. no complete cache-index entry's key names a retired family
         (substring match, case-insensitive -- the same convention
         `ci_lint`'s static CACHE-009 check uses for the same list)
    """

    entries = tuple(_read_index(store_path / "bolt.db"))
    disk_bytes = _disk_bytes(store_path)

    findings: list[str] = []
    retired_hits: list[tuple[CacheIndexEntry, str]] = []
    for entry in entries:
        key_norm = entry.key.lower()
        for family in retired_families:
            if family.lower() in key_norm:
                retired_hits.append((entry, family))
                findings.append(
                    f"{RULE}: local act cache saved key {entry.key!r} "
                    f"({format_size(entry.cache_size)}), which matches retired family "
                    f"{family!r} (ci.toml [cache].retired). Fix: remove whatever input "
                    f"re-enabled '{family}' (see CACHE-009's setup-soldr wrapper check) "
                    f"and re-run 'python3 ci/local.py act' with an empty act cache "
                    f"volume, or 'bosn gc' the machine-scoped act-cache-server volume."
                )

    if disk_bytes > budget_bytes:
        findings.append(
            f"{RULE}: local act cache store at {store_path} is {format_size(disk_bytes)}, "
            f"over ci.toml [cache].budget ({format_size(budget_bytes)}). Fix: this mirrors "
            f"what the remote budget proof (CACHE-004) checks in CI -- shrink a "
            f"[cache.family].max, or run 'bosn gc' to reclaim the machine-scoped "
            f"act-cache-server volume and start warm caching over."
        )

    return ActCacheAuditResult(
        store_path=store_path,
        disk_bytes=disk_bytes,
        budget_bytes=budget_bytes,
        entries=entries,
        retired_hits=tuple(retired_hits),
        findings=tuple(findings),
    )


def render(result: ActCacheAuditResult) -> str:
    lines = [
        f"[ci/local.py] act cache audit: {result.store_path}",
        f"  disk usage: {format_size(result.disk_bytes)} / budget {format_size(result.budget_bytes)}",
        f"  complete index entries: {len(result.entries)}",
    ]
    if result.findings:
        lines.append("  FAILED:")
        lines.extend(f"    {f}" for f in result.findings)
    else:
        lines.append("  ok: within budget, no retired-family key observed")
    return "\n".join(lines)
