#!/usr/bin/env python3
"""PreToolUse hook: blocks unsafe command shapes.

Rejects:
  - Bare `cargo` / `rustc` / `rustfmt` / `clippy-driver` / `rustup` /
    `rustdoc` — must be invoked through a build entry point or `uv run`
    with explicit project context (i.e., from `./test` or `./build`).
  - Bare `python` / `python3` — must go through `uv run`.
  - Bare `pip` / `pip3` — must go through `uv pip`.
  - `uv run` *without* `--no-project --script` for invocations outside
    the named build entry points. The whole point of `./ci.py`'s
    `--no-project --script` discipline (rule 2 of zackees/zccache#835)
    is to avoid the maturin auto-build trap when running gates; an
    agent that pastes a snippet from chat needs the protective flags
    or needs to route through a named opt-in entry point.

The hook is conservative — it only blocks command *shapes*, never
specific arguments. The named build entry points (`./test`, `./build`,
`./install`) are detected by the leading token of the about-to-run
command and are exempt. `soldr` invocations are always exempt (it's
the required entry point for compile-bearing Rust/wheel work — see
zackees/ci.yml#6 round 1).
"""

from __future__ import annotations

import json
import re
import sys

RUST_TOOLS = {
    "cargo",
    "rustc",
    "rustfmt",
    "clippy-driver",
    "cargo-clippy",
    "cargo-fmt",
    "rustup",
    "rustdoc",
    "rust-gdb",
    "rust-lldb",
    "rust-analyzer",
    # maturin is soldr's own dependency (invoked as `soldr wheel` or via
    # the soldr PEP 517 backend), never a direct entry point in this
    # repo — see zackees/ci.yml#6 round 1 (`PKG-003`).
    "maturin",
}
PYTHON_TOOLS = {"python", "python3", "pip", "pip3"}

# Routing through one of these is the documented opt-in to a full
# project sync / soldr-driven build context (rule 5 of
# zackees/zccache#835). These are detected by the leading shell token
# after env-stripping; either with or without a leading `./` and any
# extension.
#
# `build_wheel.py` and `publish.py` were removed in zackees/ci.yml#6
# round 1 (the soldr PEP 517 backend's `bundle-bins` replaced the
# former; mock publish is a later round's work — see `docs/RELEASE.md`)
# and are deliberately NOT listed here: removing an entry point only
# narrows what bypasses protection, never weakens what this hook blocks.
BUILD_ENTRY_POINTS = {
    "test",
    "build",
    "install",
    "test.py",
}

SHELL_WRAPPERS = {"cmd", "powershell", "pwsh", "bash", "sh", "zsh"}

# uv-run options that take a value (so we can correctly skip past them
# when scanning for the script positional or the protective flags).
UV_RUN_OPTIONS_WITH_VALUE = {
    "--config-file",
    "--directory",
    "--env-file",
    "--exclude-newer",
    "--extra",
    "--index",
    "--index-strategy",
    "--keyring-provider",
    "--link-mode",
    "--module",
    "--no-binary",
    "--no-binary-package",
    "--no-build-isolation-package",
    "--no-build-package",
    "--no-extra",
    "--no-group",
    "--only-group",
    "--project",
    "--python",
    "--python-platform",
    "--refresh-package",
    "--resolution",
    "--script",
    "--upgrade-package",
    "--with",
    "--with-editable",
    "--with-requirements",
}

# Flags that, when present on a `uv run` line, are considered to
# satisfy the protection requirement.
PROTECTIVE_FLAGS = {"--no-project", "--script"}

SHELL_TOOL_NAMES = {
    "Bash",
    "Shell",
    "PowerShell",
    "shell_command",
    "functions.shell_command",
}


def _extract_command(data: dict) -> str:
    tool_input = data.get("tool_input") or data.get("toolInput") or {}
    if isinstance(tool_input, str):
        return tool_input
    if not isinstance(tool_input, dict):
        return ""
    for key in ("command", "script"):
        value = tool_input.get(key)
        if isinstance(value, str):
            return value
    argv = tool_input.get("argv")
    if isinstance(argv, list):
        return " ".join(str(p) for p in argv)
    return ""


def _is_env_assignment(word: str) -> bool:
    return re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", word) is not None


def _split_segments(command: str) -> list[str]:
    out: list[str] = []
    buf: list[str] = []
    quote: str | None = None
    i = 0
    while i < len(command):
        ch = command[i]
        if quote is not None:
            buf.append(ch)
            if ch == quote:
                quote = None
            i += 1
            continue
        if ch in {"'", '"'}:
            quote = ch
            buf.append(ch)
            i += 1
            continue
        nxt = command[i + 1] if i + 1 < len(command) else ""
        if (
            ch in {";", "\r", "\n"}
            or (ch == "&" and nxt == "&")
            or (ch == "|" and nxt == "|")
            or ch == "|"
        ):
            segment = "".join(buf).strip()
            if segment:
                out.append(segment)
            buf = []
            i += 2 if ((ch == "&" and nxt == "&") or (ch == "|" and nxt == "|")) else 1
            continue
        buf.append(ch)
        i += 1
    seg = "".join(buf).strip()
    if seg:
        out.append(seg)
    return out


def _tokenize(segment: str) -> list[str]:
    words: list[str] = []
    buf: list[str] = []
    quote: str | None = None
    for ch in segment:
        if quote is not None:
            if ch == quote:
                quote = None
            else:
                buf.append(ch)
            continue
        if ch in {"'", '"'}:
            quote = ch
            continue
        if ch.isspace():
            if buf:
                words.append("".join(buf))
                buf = []
            continue
        buf.append(ch)
    if buf:
        words.append("".join(buf))
    return words


def _program_name(word: str) -> str:
    cleaned = word.strip().strip("'\"").replace("\\", "/")
    while cleaned.startswith("./"):
        cleaned = cleaned[2:]
    base = cleaned.rsplit("/", 1)[-1].lower()
    for suffix in (".exe", ".cmd", ".bat", ".ps1", ".sh"):
        if base.endswith(suffix):
            base = base[: -len(suffix)]
            break
    return base


def _strip_env_prefix(words: list[str]) -> list[str]:
    while words and words[0] in {"&", "call", "exec", "command"}:
        words = words[1:]
    if words and _program_name(words[0]) == "env":
        words = words[1:]
    while words and _is_env_assignment(words[0]):
        words = words[1:]
    return words


def _is_named_build_entry(words: list[str]) -> bool:
    if not words:
        return False
    head = _program_name(words[0])
    if head in BUILD_ENTRY_POINTS:
        return True
    # `uv run --no-project --script ci/gates/test.py` — only scoped to
    # uv invocations so that bare `cargo build` doesn't pass just because
    # `build` is a named entry point.
    if head == "uv" and len(words) > 1 and words[1] == "run":
        for w in words[2:]:
            if _program_name(w) in BUILD_ENTRY_POINTS:
                return True
    return False


def _uv_run_has_protection(words: list[str]) -> bool:
    if len(words) < 2 or _program_name(words[0]) != "uv" or words[1] != "run":
        return False
    seen = set()
    i = 2
    while i < len(words):
        w = words[i]
        if w == "--":
            break
        if w in PROTECTIVE_FLAGS:
            seen.add(w)
        if "=" in w:
            base = w.split("=", 1)[0]
            if base in PROTECTIVE_FLAGS:
                seen.add(base)
        i += 1
    return PROTECTIVE_FLAGS.issubset(seen)


def _nested_shell(words: list[str]) -> str | None:  # noqa: C901
    if not words:
        return None
    head = _program_name(words[0])
    if head not in SHELL_WRAPPERS:
        return None
    if head == "cmd":
        for i, w in enumerate(words[1:], start=1):
            if w.lower() in {"/c", "/r"} and i + 1 < len(words):
                return " ".join(words[i + 1 :])
        return None
    if head in {"powershell", "pwsh"}:
        for i, w in enumerate(words[1:], start=1):
            if w.lower() in {"-command", "-c", "/c"} and i + 1 < len(words):
                return " ".join(words[i + 1 :])
        return None
    for i, w in enumerate(words[1:], start=1):
        opt = w.lower().lstrip("-")
        if "c" in opt and i + 1 < len(words):
            return " ".join(words[i + 1 :])
    return None


def _check_segment(seg: str) -> tuple[str, str] | None:  # noqa: C901
    words = _strip_env_prefix(_tokenize(seg))
    if not words:
        return None

    nested = _nested_shell(words)
    if nested is not None:
        for sub in _split_segments(nested):
            result = _check_segment(sub)
            if result:
                return result
        return None

    head = _program_name(words[0])

    # Routing through a named build entry point: full pass — those scripts
    # are the documented opt-in to the maturin context.
    if _is_named_build_entry(words):
        return None

    if head == "soldr":
        return None

    if head == "uv":
        if len(words) > 1 and words[1] == "run":
            if _uv_run_has_protection(words):
                return None
            return (
                "uv run",
                "Use `./ci.py <gate>` for lint/gate invocations, or run "
                "your build through a named entry point (./test, ./install). "
                "Bare `uv run` walks up to pyproject.toml "
                "and triggers the wheel build (soldr PEP 517 backend) "
                "before your script "
                "starts. Add `--no-project --script` to skip discovery and "
                "use the PEP 723 inline-deps path.",
            )
        # uv pip / uv tool / etc. are fine.
        return None

    if head in RUST_TOOLS:
        return (
            head,
            f"Use `./ci.py <gate>` (clippy/fmt/build/test) or route through a named build entry point. Bare `{head}` bypasses the workspace's pinned toolchain configuration.",
        )

    if head in PYTHON_TOOLS:
        if head.startswith("pip"):
            return (
                head,
                f"Use `uv pip ...` instead of bare `{head}`. All pip operations must go through uv so the lock file stays authoritative.",
            )
        return (
            head,
            f"Use `uv run ...` (with `--no-project --script` for gates, or a named build entry point for builds) instead of bare `{head}`. All Python must be executed through uv.",
        )

    return None


def check_command(command: str) -> tuple[str, str] | None:
    for seg in _split_segments(command):
        result = _check_segment(seg)
        if result:
            return result
    return None


def deny(reason: str) -> None:
    json.dump(
        {
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        },
        sys.stdout,
    )


def main() -> None:
    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError:
        sys.exit(0)

    if data.get("tool_name", "") not in SHELL_TOOL_NAMES:
        sys.exit(0)

    command = _extract_command(data)
    if not command:
        sys.exit(0)

    result = check_command(command)
    if result:
        _, reason = result
        deny(reason)
        print(reason, file=sys.stderr)
        sys.exit(2)

    sys.exit(0)


if __name__ == "__main__":
    main()
