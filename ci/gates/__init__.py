"""CI gates: one `run() -> int` per file.

Each module under `ci.gates.*` exposes a single zero-arg `run()` returning
the conventional Unix exit code (0 = pass, non-zero = fail). The canonical
ordering is owned by `ci.py::GATE_ORDER`, NOT by alphabetical name.

Hooks vs gates — repo-state vs agent-intent:
  - Gates here run on every push (GHA `./ci.py all`) and on developer
    laptops. They check repo state.
  - Hooks under `ci/hooks/` only fire during a Claude/Codex session
    (PreToolUse / PostToolUse / SessionStart). They check agent intent
    at the moment of an action.

If a rule would fire equally well on a `git push` from a terminal as
from a Claude edit, write it as a gate here. If it needs to know what
tool is about to run, what file is being written, or what session just
started, write it under `ci/hooks/`.
"""
