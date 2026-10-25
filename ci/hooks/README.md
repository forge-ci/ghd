# `ci/hooks/`

Agent-intent guards that fire **only during a Claude/Codex session** —
PreToolUse blocks a forbidden command shape before it runs, PostToolUse
reacts to a file edit, SessionStart captures a fingerprint. None of
them run as part of CI on a `git push`.

## Why these are hooks and not gates

Repo-state checks belong in `ci/gates/` because they fire equally on
every CI cycle regardless of where the change came from. Hooks here
need information that only exists at the moment the agent is acting —
which tool is about to run, which file just got edited, when the
session opened. See [zccache#835 rule 9](https://github.com/zackees/zccache/issues/835).

| Concern                                | Home |
|----------------------------------------|------|
| File size budget across the workspace  | `ci/gates/loc.py` |
| File size growth on this edit          | `ci/hooks/loc_guard.py` |
| README presence after a new file lands | `ci/hooks/readme_guard.py` |
| Bare `cargo` / `python` / unsafe `uv run` | `ci/hooks/tool_guard.py` |
| Git fingerprint at session start       | `ci/hooks/check-on-start.py` |
| Contract precheck (workflows/ci.toml/manifests) | `ci/hooks/local_precheck_guard.py` |

## Hooks in this directory

- `tool_guard.py` — **PreToolUse**. Reads the about-to-run Bash command
  payload, rejects bare `cargo`/`rustc`/`rustfmt`/`maturin`/`python`/`pip`,
  and rejects `uv run` without `--no-project --script` outside the
  named build entry points (`./test`, `./build`, `./install`). `soldr`
  invocations always pass — it's the required entry point for
  compile-bearing Rust/wheel work (zackees/ci.yml#6 round 1). Returns
  exit 2 with a structured deny payload so Claude Code surfaces the
  reason to the agent.

- `readme_guard.py` — **PostToolUse on Edit|Write**. After any file
  edit, checks the containing directory for a `README.md` of at least
  50 lines. Missing or too short → exit 2 with a prescription
  ("expand it with what's in this directory + why + key entry points").

- `loc_guard.py` — **PostToolUse on Edit|Write**. After any source-file
  edit, counts the resulting LOC. Warns over 1000, hard-blocks over
  1500 with the canonical split convention.

- `check-on-start.py` — **SessionStart**. Captures an MD5 fingerprint of
  `git status --porcelain` into `.cache/session_fingerprint.json` so a
  future Stop hook can decide whether anything changed during the
  session.

- `local_precheck_guard.py` — **PostToolUse on Edit|Write|MultiEdit**,
  and **Stop** (with `--stop`). Runs `python3 ci/local.py precheck`
  (zackees/ci.yml#6 §11's local gate) whenever the edited file is under
  `.github/**`, or is `ci.toml`/`bosn.toml`/`Cargo.toml`/`pyproject.toml`/
  `action.yml`; the Stop invocation runs it unconditionally, as a
  backstop for edits `local_precheck_guard.py` couldn't see (e.g. one
  made through `Bash`). Missing/too-short READMEs are `readme_guard.py`'s
  job, not this hook's. Closes zackees/zccache#1760's root causes 1 and
  2 for this repository: the guard now runs locally, and is never
  blocked by `tool_guard.py` (a plain `python3 <script>` call always
  passes that hook).

## Wiring

`.claude/settings.json` at the repo root binds each hook to its event.
The full list is canonical; downstream forks should keep the binding
shape and only customize the hook body. See the
[Claude Code hooks docs](https://docs.claude.com/en/docs/claude-code/hooks)
for the schema.

## Hook contract

Each hook reads a JSON payload from stdin and exits:

- `0`: pass / not applicable.
- `2`: block; the stderr message is fed back to Claude so the agent
  sees *why* and can self-correct without the user intervening.

Stdout is reserved for the structured permission decision (PreToolUse
only) — see `tool_guard.py::deny()` for the shape.
