"""ghd demo: real commands against real GitHub (talos-hq/clud).

Recorded with script(1); see README.md. Uses the repo's .venv ghd.
"""

import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
R = "talos-hq/clud"
PRS = "2135 2133 2132 2131 2128 2135 2133 2132 2131 2128"
FAN_OUT = (
    f"printf '%s\\n' {PRS} | GHD_DEBUG=1 xargs -P10 -I{{}} "
    f"ghd pr view {{}} -R {R} --json state 2>&1 >/dev/null | sort | uniq -c"
)
WARM = f"time GHD_DEBUG=1 ghd pr view 2138 -R {R} --json state"
PY = (
    'python -c "from ghd.bindings import pr_view; '
    f"print(pr_view('{R}', 2138, ['state', 'mergeable']))\""
)

SCRIPT = [
    ("say", "ghd: a cached, batched gateway for gh reads"),
    ("cmd", "ghd --version"),
    ("say", "cold: the first read starts the daemon and fetches over GraphQL"),
    ("cmd", f"time ghd pr view 2138 -R {R} --json number,state,title,mergedAt"),
    ("say", "10 agents at once: 5 PRs, each asked twice"),
    ("cmd", FAN_OUT),
    ("cmd", "ghd stats"),
    ("say", "meanwhile, the same read through plain gh"),
    ("cmd", f"time gh pr view 2138 -R {R} --json state >/dev/null"),
    ("say", "the 10 agents ask again: freshness proven by free ETag 304s"),
    ("cmd", FAN_OUT),
    ("cmd", "ghd stats"),
    (
        "say",
        "warm reads: the first revalidates (one REST round trip), the next is served fresh",
    ),
    ("cmd", WARM),
    ("cmd", WARM),
    ("say", "Python callers share the same cache"),
    ("cmd", PY),
]


def main() -> None:
    env = dict(os.environ, GHD_FRESH_MS="400")
    env["PATH"] = f"{ROOT / '.venv' / 'bin'}{os.pathsep}{env['PATH']}"
    for kind, text in SCRIPT:
        if kind == "say":
            print(f"\n\033[1;36m# {text}\033[0m", flush=True)
        else:
            # \001 / \002 mark the command so to_cast.py can type it out.
            print(f"\033[1;32m$\033[0m \001{text}\002", flush=True)
            subprocess.run(["bash", "-c", text], env=env, check=False)


if __name__ == "__main__":
    main()
