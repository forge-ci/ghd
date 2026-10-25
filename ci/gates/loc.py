"""Workspace LOC budget gate.

Walks every tracked source file under the repo (excluding generated /
vendored / build-output trees) and enforces:
    LOC > 1000  -> warning  (gate still passes)
    LOC > 1500  -> failure  (gate returns 1)

Refactor convention printed on every offender:
    foo.rs -> foo/mod.rs + per-domain submodules, `pub use` re-exports
              in mod.rs so the public path is unchanged.

This is a gate, not a hook — it walks the whole tree on every CI run, so
a file that grew past budget via `git push` (no agent in the loop) is
caught the same way an agent edit would be.
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

WARN = 1000
ERROR = 1500

SOURCE_EXTS = {
    ".rs",
    ".py",
    ".ts",
    ".tsx",
    ".js",
    ".jsx",
    ".go",
    ".java",
    ".kt",
    ".swift",
    ".c",
    ".cc",
    ".cpp",
    ".cxx",
    ".h",
    ".hh",
    ".hpp",
}

EXCLUDED = {
    ".git",
    ".ci-lint",  # Downloaded third-party checker, not repository source.
    "target",
    ".cargo",
    ".rustup",
    ".venv",
    "node_modules",
    "__pycache__",
    "dist",
    "build",
    ".claude",
    ".pytest_cache",
    ".ruff_cache",
    ".mypy_cache",
    ".cache",
}


def _line_count(path: Path) -> int:
    with path.open("rb") as fh:
        return sum(1 for _ in fh)


def _iter_source_files() -> list[Path]:
    out: list[Path] = []
    for path in ROOT.rglob("*"):
        if not path.is_file():
            continue
        if path.suffix.lower() not in SOURCE_EXTS:
            continue
        if any(part in EXCLUDED for part in path.parts):
            continue
        out.append(path)
    return out


def run() -> int:
    warns: list[tuple[Path, int]] = []
    fails: list[tuple[Path, int]] = []
    for path in _iter_source_files():
        try:
            loc = _line_count(path)
        except OSError:
            continue
        if loc > ERROR:
            fails.append((path, loc))
        elif loc > WARN:
            warns.append((path, loc))

    if warns:
        print(f"LOC warnings (>{WARN}):")
        for path, loc in warns:
            rel = path.relative_to(ROOT)
            print(f"  WARN  {rel}  {loc} lines")

    if fails:
        print(f"LOC failures (>{ERROR}):")
        for path, loc in fails:
            rel = path.relative_to(ROOT)
            print(f"  FAIL  {rel}  {loc} lines")
        print(
            "\nRefactor convention: foo.rs -> foo/mod.rs + per-domain "
            "submodules, with `pub use` re-exports in mod.rs so the public "
            "path is unchanged. Target < 1000 lines per file so future edits "
            "have headroom."
        )
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(run())
