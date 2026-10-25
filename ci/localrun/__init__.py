"""Local CI loop: bosn -> act, and the agent precheck gate.

zackees/ci.yml#6 section 11. This package backs the single `ci/local.py`
entry point (`python3 ci/local.py precheck|act`). It is imported both on
the host (precheck, event planning, bosn orchestration) and inside the
bosn `act` stack container (`act-inner`, which drives the pinned `act`
binary and audits its local cache store). Every module here is Python
standard library only -- no PyPI dependency, matching `ci_lint`'s own
stdlib-only rule (AGENTS.md) -- except where a module explicitly shells
out to `uv run --no-project --with pyyaml` for `ci_lint` itself, which
needs PyYAML.
"""

from __future__ import annotations
