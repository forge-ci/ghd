"""Host-platform facade for the Python side of this repo.

Every host check — Windows vs. Linux vs. macOS, native executable
suffix — lives here and nowhere else. This mirrors the Rust
`ghd-platform` facade (`crates/private/ghd-platform`): one
neutral surface with real, host-specific answers, no `sys.platform`,
`os.name`, or `platform.system()` call anywhere else in the tree.

`ci.toml`'s `[allow] platform-code` lists two allowed locations for
native/host-branching code in this workspace:
`crates/private/ghd-platform/src/platforms/**` (Rust) and
`src/ghd/platforms/**` (this package, Python).
Nothing else may call `sys.platform` / `os.name` / `platform.system()`
— that includes test files and the `ci/` build scripts, which import
this module the same way application code would (it has zero runtime
dependencies, so importing it never requires the `_native` extension
to be built).

Deliberately small: like the Rust side, this proves the boundary shape
with one real capability (executable suffix / OS name) rather than a
full platform SDK. Grow it here, not with an inline check somewhere
else, when a second one is needed.
"""

from __future__ import annotations

import sys

__all__ = [
    "binary_suffix",
    "cli_binary_name",
    "is_linux",
    "is_macos",
    "is_windows",
    "os_name",
]


def is_windows() -> bool:
    """True when running on Windows."""
    return sys.platform == "win32"


def is_linux() -> bool:
    """True when running on Linux."""
    return sys.platform.startswith("linux")


def is_macos() -> bool:
    """True when running on macOS."""
    return sys.platform == "darwin"


def os_name() -> str:
    """The running host's OS name: `"windows"`, `"linux"`, or `"macos"`.

    Mirrors `ghd_platform::host::os_name()` on the Rust side.
    Raises `RuntimeError` on an unsupported host rather than guessing —
    same "no silent fallback" rule as the Rust `cfg_select!` selector.
    """
    if is_windows():
        return "windows"
    if is_linux():
        return "linux"
    if is_macos():
        return "macos"
    raise RuntimeError(f"unsupported host platform: {sys.platform!r}")


def binary_suffix() -> str:
    """Suffix appended to native executables on this host.

    `".exe"` on Windows, empty everywhere else — the same convention
    `ghd_platform::host::exe_suffix()` proves on the Rust side.
    """
    return ".exe" if is_windows() else ""


def cli_binary_name() -> str:
    """Expected filename of the native `ghd` binary on this host."""
    return f"ghd{binary_suffix()}"
