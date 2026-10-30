"""`python3 ci/instantiate.py --name <kebab-name> --out <dir>`

zackees/ci.yml#6 round-4B, the `[ci-test-init]` suite: copies this
repository into `--out`, renaming every "template"-derived crate,
Python-package and CLI identifier to `--name`, so the `init` suite can
prove this repo is a genuinely REUSABLE template -- not just a fixed
demo that happens to build -- from a fresh instantiation with no warm
cache and no leftover state from the template's own identity.

Scope (deliberate): renames every STRUCTURAL identifier a build or
`ci_lint precheck` run depends on --

  - crate directory names (`crates/template*` -> `crates/<name>*`,
    `crates/private/template-*` -> `crates/private/<name>-*`)
  - every `Cargo.toml`'s `name`/`[lib].name`/`[[bin]].name` and
    path-dependency table keys + `path =` values
  - `Cargo.lock` package entries with a matching `name` (and their
    `dependencies` cross-references) -- textual (the same compound- and
    bare-identifier passes used everywhere else), not `cargo`-
    regenerated: every renamed package is a local path member with no
    `checksum` field to invalidate, so a plain string substitution keeps
    the lockfile internally consistent without needing network access
  - `crates/template`'s own doctest/integration-test files, which
    reference it by Rust path (`template::...` -> `<name_snake>::...`,
    the crate's own Rust *identifier* -- note this differs from its
    Cargo.toml *package name* whenever `--name` is hyphenated, exactly
    the way Cargo/rustc treat every hyphenated crate name)
  - the Python package directory (`src/ghd` ->
    `src/<name_snake>`) and every `pyproject.toml`/`ci.toml` field that
    names it, the CLI, or the amalgam crate

Deliberately does NOT rewrite free-form prose/comments describing "the
template" generically (README, doc comments, this repo's own
case-study-style commentary): correct build/precheck/smoke behavior
does not depend on comment wording, and a blind global replace of the
bare word "template" would corrupt prose throughout the copied tree
with no functional benefit -- see `BARE_TEMPLATE_TOML_ONLY` below for
where a bare (non-compound) "template" token IS renamed, and why that
is scoped to `Cargo.toml`/`ci.toml` specifically, not applied tree-wide.

Deliberately does NOT attempt to rename `dylints/platform_boundary`'s
own self-test fixture paths (`crates/template/tests/host.rs` literals
in its `#[test]` list) -- those exercise the LINT's own baseline against
ITS OWN crate tree, not the generated demo's; the `init` suite does not
run Dylint (that is the separate `dylint` job).
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Never copied into --out: build/tool state, VCS metadata, anything the
# fresh instantiation must NOT inherit (the whole point of `init` is
# proving a build from zero -- ci.toml [suites].init `cache = "none"`).
EXCLUDE_DIR_NAMES = frozenset(
    {
        ".git",
        "target",
        ".venv",
        ".venv-smoke",
        "dist",
        "__pycache__",
        ".pytest_cache",
        ".ruff_cache",
        "node_modules",
        ".ci-lint",
        ".act-local",
        ".cargo",  # crates/private/dylints's own .cargo/registry vendor cache
    }
)

# Ordered longest-compound-first so e.g. "ghd" is
# consumed whole before "ghd-py" could match a substring of it.
# {kebab}/{snake} are filled in by `_replacements()`.
_COMPOUND_ORDER = [
    "ghd-cross-pure",
    "ghd_cross_pure",
    "ghd-platform",
    "ghd_platform",
    "ghd-core",
    "ghd_core",
    "ghd-daemon",
    "ghd_daemon",
    "ghd-cli",
    "ghd_cli",
    "ghd-py",
    "ghd_py",
]

# Files where a BARE "template" token (not part of any compound above)
# is a structural identifier, not prose -- verified by direct inspection
# of this repository (round-4B worker report): every non-compound
# "template" occurrence in a *.toml file is one of Cargo.toml's
# `name = "template"` / `[lib] name = "template"`, the root Cargo.toml's
# `"crates/template"` workspace member, or ci.toml's `public =
# "crates/template"` / `"template:test:api"`. No other tracked file has
# a bare structural "template" outside these compounds and Rust `use`/
# path expressions (handled separately, see `_RUST_BARE_TEMPLATE_FILES`).
_BARE_TEMPLATE_GLOBS = ("Cargo.toml", "Cargo.lock", "ci.toml", "pyproject.toml")

# Every consumer of the amalgam crate (its own doctest, its integration
# test, the CLI binary, the PyO3 extension) reaches it by bare Rust path
# (`template::...`) -- the crate's *Rust identifier*, which is `--name`'s
# snake_case form regardless of the Cargo.toml package name's (possibly
# hyphenated) kebab form. Scanned tree-wide (every `.rs` file under
# `crates/`, not a fixed enumeration): a real instantiated build
# (round-4B worker report) found `template::` in FOUR files
# (`crates/template/{src/lib.rs,tests/api/main.rs}`,
# `crates/ghd-cli/src/main.rs`, `crates/ghd-py/src/lib.rs`),
# not just the amalgam crate's own two -- a fixed list is exactly the
# kind of enumeration a new consumer silently falls outside of.


def _kebab(name: str) -> str:
    return name.strip().lower().replace("_", "-")


def _snake(name: str) -> str:
    return name.strip().lower().replace("-", "_")


def _replacements(name: str) -> list[tuple[str, str]]:
    kebab = _kebab(name)
    snake = _snake(name)
    out: list[tuple[str, str]] = []
    for token in _COMPOUND_ORDER:
        sep = snake if "_" in token else kebab
        out.append((token, token.replace("ghd", sep, 1)))
    return out


# Bare `ghd` (no compound suffix) is ambiguous: the kebab form in manifests
# (`name = "ghd"`, `crates/ghd`), the snake form for the Python package and
# the Rust crate identifier (`src/ghd/`, `from ghd`, `ghd::`). Prose, URLs
# (`forge-ci/ghd`) and the `ghd.rs` file name are deliberately left alone.
_SNAKE_CONTEXTS = (
    re.compile(r"(?<=src/)ghd\b(?!\.rs)"),
    re.compile(r"(?<=\bfrom )ghd\b"),
    re.compile(r"(?<=\bimport )ghd\b"),
    re.compile(r"\bghd(?=\.(?:bindings|platforms|_native|__version__)\b)"),
    re.compile(r"\bghd(?=::)"),
)
_KEBAB_PATH_CONTEXTS = (
    re.compile(r"(?<=crates/)ghd\b(?!-)"),
    re.compile(r"(?<=\.\./)ghd\b(?!-)"),
)
# In manifests only: a bare token that is not part of a URL/path/compound.
_BARE_TOML = re.compile(r"(?<![/\w.-])ghd\b(?![-.\w/])")


def _should_skip_dir(name: str) -> bool:
    return name in EXCLUDE_DIR_NAMES


# A `.cargo/` dir's vendor/registry state is excluded, but its checked-in
# config is source: a Dylint lint crate's `.cargo/config.toml` sets
# `linker=dylint-link`, without which cargo-dylint never finds the
# toolchain-stamped `lib<name>@<toolchain>.so` ("despite successful build").
_CARGO_CONFIG_FILES = ("config.toml", "config")


def _copy_cargo_config(src: Path, dst: Path) -> None:
    for name in _CARGO_CONFIG_FILES:
        cfg = src / name
        if cfg.is_file() and not cfg.is_symlink():
            dst.mkdir(parents=True, exist_ok=True)
            shutil.copy2(cfg, dst / name)


def _copy_tree(src: Path, dst: Path) -> None:
    if dst.exists():
        raise InstantiateError(f"--out {dst} already exists; refusing to overwrite")
    dst.mkdir(parents=True)
    for entry in sorted(src.iterdir()):
        if entry.is_dir():
            if entry.name == ".cargo":
                _copy_cargo_config(entry, dst / entry.name)
                continue
            if _should_skip_dir(entry.name):
                continue
            _copy_tree(entry, dst / entry.name)
        elif entry.is_symlink():
            continue  # never present in this repo's tracked tree; skip defensively
        else:
            shutil.copy2(entry, dst / entry.name)


def _is_text_file(path: Path) -> bool:
    if path.suffix in {
        ".png",
        ".jpg",
        ".jpeg",
        ".ico",
        ".whl",
        ".tar",
        ".gz",
        ".zst",
        ".so",
        ".pyd",
        ".dll",
    }:
        return False
    try:
        path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError):
        return False
    return True


def _apply_compound_replacements(
    out_dir: Path, replacements: list[tuple[str, str]], kebab: str, snake: str
) -> int:
    changed = 0
    for path in sorted(out_dir.rglob("*")):
        if path.is_dir() or not _is_text_file(path):
            continue
        text = path.read_text(encoding="utf-8")
        new_text = text
        for old, new in replacements:
            new_text = new_text.replace(old, new)
        for pattern in _SNAKE_CONTEXTS:
            new_text = pattern.sub(snake, new_text)
        for pattern in _KEBAB_PATH_CONTEXTS:
            new_text = pattern.sub(kebab, new_text)
        if new_text != text:
            path.write_text(new_text, encoding="utf-8")
            changed += 1
    return changed


def _apply_bare_template_toml(out_dir: Path, kebab: str) -> int:
    pattern = _BARE_TOML
    changed = 0
    for glob in _BARE_TEMPLATE_GLOBS:
        for path in sorted(out_dir.rglob(glob)):
            text = path.read_text(encoding="utf-8")
            new_text = pattern.sub(kebab, text)
            if new_text != text:
                path.write_text(new_text, encoding="utf-8")
                changed += 1
    return changed


def _apply_bare_template_rust(out_dir: Path, snake: str) -> int:
    # `template::` (the Rust path-separator form), not bare `\btemplate\b`
    # -- deliberately narrower than the Cargo.toml/ci.toml pass above, so
    # this never touches a `.rs` file's PROSE that happens to mention
    # "the template" in a doc comment (e.g. dylints/platform_boundary's
    # own commentary) -- only actual Rust path expressions, which is
    # exactly the crate's usage in doctests/integration tests/consumers.
    pattern = re.compile(r"\bghd::")
    changed = 0
    crates_dir = out_dir / "crates"
    if not crates_dir.is_dir():
        return 0
    for path in sorted(crates_dir.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        new_text = pattern.sub(f"{snake}::", text)
        if new_text != text:
            path.write_text(new_text, encoding="utf-8")
            changed += 1
    return changed


def _fix_lib_target_name(out_dir: Path, kebab: str, snake: str) -> None:
    """`crates/<kebab>/Cargo.toml`'s `[lib] name = "..."` must be the
    SNAKE identifier, never kebab: unlike `[package].name` (Cargo allows
    hyphens there) or `[[bin]].name` (a binary's on-PATH filename, also
    hyphen-safe), `[lib].name` becomes a Rust `extern crate`/`use`
    identifier, where a hyphen is a syntax error. The general bare-
    "template" pass (`_apply_bare_template_toml`) does not distinguish
    `[package]` from `[lib]` and renamed both to the kebab form --
    confirmed to break `soldr cargo check` on a real instantiated tree
    (round-4B worker report: "library target names cannot contain
    hyphens"). This targeted fix runs after that pass and only touches
    the `[lib]` table's own `name` line."""

    manifest = out_dir / "crates" / kebab / "Cargo.toml"
    if not manifest.is_file():
        return
    text = manifest.read_text(encoding="utf-8")
    lines = text.splitlines(keepends=True)
    in_lib_table = False
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("["):
            in_lib_table = stripped == "[lib]"
            continue
        if in_lib_table and stripped.startswith("name"):
            lines[i] = line.replace(f'"{kebab}"', f'"{snake}"')
    manifest.write_text("".join(lines), encoding="utf-8")


def _rename_paths(out_dir: Path, kebab: str, snake: str) -> None:
    # Deepest-first so a parent rename never invalidates a child Path
    # object still pointing at the pre-rename name.
    dir_renames = {
        out_dir / "crates" / "private" / "ghd-platform": out_dir
        / "crates"
        / "private"
        / f"{kebab}-platform",
        out_dir / "crates" / "private" / "ghd-daemon": out_dir
        / "crates"
        / "private"
        / f"{kebab}-daemon",
        out_dir / "crates" / "private" / "ghd-core": out_dir
        / "crates"
        / "private"
        / f"{kebab}-core",
        out_dir / "crates" / "private" / "ghd-cross-pure": out_dir
        / "crates"
        / "private"
        / f"{kebab}-cross-pure",
        out_dir / "crates" / "ghd-py": out_dir / "crates" / f"{kebab}-py",
        out_dir / "crates" / "ghd-cli": out_dir / "crates" / f"{kebab}-cli",
        out_dir / "crates" / "ghd": out_dir / "crates" / kebab,
        out_dir / "src" / "ghd": out_dir / "src" / snake,
    }
    for old, new in dir_renames.items():
        if old.is_dir():
            old.rename(new)


class InstantiateError(RuntimeError):
    pass


def run(args: argparse.Namespace) -> int:
    name = args.name
    if not re.fullmatch(r"[a-z][a-z0-9]*(-[a-z0-9]+)*", _kebab(name)):
        print(
            f"ci/instantiate.py: --name {name!r} must be lowercase "
            "alphanumeric words separated by '-' or '_' (e.g. 'demo-app')",
            file=sys.stderr,
        )
        return 2
    kebab = _kebab(name)
    snake = _snake(name)
    out_dir = Path(args.out).resolve()

    try:
        print(f"ci/instantiate.py: copying {ROOT} -> {out_dir}", file=sys.stderr)
        _copy_tree(ROOT, out_dir)
    except InstantiateError as exc:
        print(f"ci/instantiate.py: {exc}", file=sys.stderr)
        return 1

    replacements = _replacements(name)
    n1 = _apply_compound_replacements(out_dir, replacements, kebab, snake)
    n2 = _apply_bare_template_toml(out_dir, kebab)
    n3 = _apply_bare_template_rust(out_dir, snake)
    _rename_paths(out_dir, kebab, snake)
    _fix_lib_target_name(out_dir, kebab, snake)

    print(
        f"ci/instantiate.py: renamed 'template' -> {kebab!r} (Rust/Cargo) / "
        f"{snake!r} (Python) -- {n1} file(s) via compound identifiers, "
        f"{n2} via bare-'template' in Cargo.toml/ci.toml, {n3} via bare-"
        f"'template' in the amalgam crate's own Rust source",
        file=sys.stderr,
    )
    print(f"out={out_dir}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--name", required=True, help="new project name, e.g. 'demo-app'"
    )
    parser.add_argument(
        "--out", required=True, help="destination directory (must not exist)"
    )
    args = parser.parse_args(argv)
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
