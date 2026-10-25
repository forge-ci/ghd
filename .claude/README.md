# `.claude/`

Per-project configuration for Claude Code. Right now this directory
just holds `settings.json`, which wires the four canonical hooks
defined in `ci/hooks/`.

## What's wired

`settings.json` binds each hook to its event:

| Event       | Hook                              | Matcher        |
|-------------|-----------------------------------|----------------|
| PreToolUse  | `ci/hooks/tool_guard.py`          | Bash/Shell/PS  |
| PostToolUse | `ci/hooks/readme_guard.py`        | Edit/Write     |
| PostToolUse | `ci/hooks/loc_guard.py`           | Edit/Write     |
| SessionStart| `ci/hooks/check-on-start.py`      | any            |

All hooks are invoked through `uv run --no-project --script`, the
same discipline the rest of the repo follows. This avoids the maturin
auto-build trap that would otherwise fire on every hook execution
(see `ci.py` for the rationale).

## Why hooks live in the repo, not user settings

Two reasons:

1. **Consistency.** Every contributor sees the same gates fire. The
   hook bodies are version-controlled, reviewed in PRs, and evolve
   with the project. User-level hook configuration is fine for
   personal helpers but doesn't survive a fresh checkout or a
   teammate joining.

2. **Hooks ARE the contract.** A bare `cargo build` triggering soldr
   discovery rules, a `python` invocation routing through `uv`, the
   LOC budget — these are project policy, not user preference. They
   belong with the source they protect.

## What's intentionally NOT here

- **Slash commands / skills.** Those live in `~/.claude/skills/` or
  `~/.claude/commands/` per user. If a project ever needs a skill
  bundled, this directory is a fine place to put it (`.claude/skills/`
  is picked up by Claude Code automatically).

- **Custom agents.** Same as skills — user-level by default.

- **API keys or secrets.** Never. Use env vars or a secret manager.

## Settings.json discipline

If you add a new hook to `ci/hooks/<name>.py`, register it here. If
you remove one, remove the binding. The
[`fewer-permission-prompts` skill](https://docs.claude.com/en/docs/claude-code/skills)
can help if local development hits permission churn — keep additions
under PreToolUse-allowlist, not under hook bypass.

For the full hook schema, see the [Claude Code hooks
documentation](https://docs.claude.com/en/docs/claude-code/hooks).
