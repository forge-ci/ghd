"""Install the aarch64 Linux cross gcc the Dylint lane needs for `ring`.

Linux TLS is rustls, whose `ring` build script compiles C even under a
check-only pass, so the aarch64 Linux Dylint pass needs a real cross gcc.

The apt lists are refreshed first: hosted runners happen to ship an index,
but the local gate's runner image does not, and a bare `apt-get install`
there fails with "Unable to locate package". One `run:` line per step
(`GEN-005`) is why this is a script rather than two shell commands.
"""

from __future__ import annotations

import subprocess

PACKAGES = ("gcc-aarch64-linux-gnu", "libc6-dev-arm64-cross")


def main() -> int:
    subprocess.run(["sudo", "apt-get", "update"], check=True)
    subprocess.run(
        ["sudo", "apt-get", "install", "-y", "--no-install-recommends", *PACKAGES],
        check=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
