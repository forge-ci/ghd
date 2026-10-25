# `ci/gates/`

Workspace-state checks that run on every CI cycle and on developer
laptops via `./ci.py <gate>`. One Python file per gate; each file
exposes a single `def run() -> int` returning the conventional Unix
exit code (0 = pass, non-zero = fail).

## Why gates live here (and not as inline YAML)

Embedded YAML shell snippets are untestable, unlintable, and
unreviewable. Pulling each step into `ci/gates/<name>.py` makes the
contract:

- **Lintable** — ruff and pyright understand `def run() -> int`
  directly. Embedded `run:` blocks are opaque.
- **Testable** — `tests/test_gates.py` can `import ci.gates.fmt;
  ci.gates.fmt.run()` against a worktree fixture and assert the exit
  code.
- **Locally reproducible** — `./ci.py fmt` on a dev laptop runs the
  *exact* same bytes as the GHA step that calls `./ci.py fmt`.
- **Replaceable in isolation** — downstream consumers (forks of this
  template) can swap one gate without forking the whole workflow file.

See [`zackees/zccache#835` rule 6](https://github.com/zackees/zccache/issues/835)
for the full rationale.

## Registered gates and ordering

The canonical run order is owned by `ci.py::GATE_ORDER`, not by the
filesystem order of this directory. The order roughly reflects cost.
Every Rust/wheel command goes through `soldr` — never bare
`cargo`/`maturin` (zackees/ci.yml#6 round 1, `RUST-001`/`PKG-003`):

1. `loc` — workspace LOC budget (cheap directory walk).
2. `fmt` — `soldr cargo fmt --check`.
3. `clippy` — `soldr cargo clippy --workspace --all-targets --locked -D warnings`.
4. `ruff` — Python linting + format check.
5. `build` — `soldr cargo check --workspace --locked` (**FATAL**: a
   failing build short-circuits the rest of the run since downstream
   gates against an uncompiled tree only emit noise).
6. `test` — `soldr cargo test --workspace --locked` + `uv sync`
   (installs the project through the soldr PEP 517 backend) + `pytest`.
7. `backend_smoke` — `uv build --wheel` through the soldr backend
   (Linux-only; see the gate's docstring for why).
8. `action_yaml` — static parse of the composite action contract.
9. `action_surface` — runtime check that subcommands referenced from
   `action.yml` exist in the built binary.

## Gates vs. hooks

| Concern                                | Home               |
|----------------------------------------|--------------------|
| File size budget (workspace-wide)      | `ci/gates/loc.py`  |
| README presence (per-edit reaction)    | `ci/hooks/readme_guard.py` |
| Tool command shape (per-edit guard)    | `ci/hooks/tool_guard.py`   |
| `fmt` / `clippy` / `ruff` / `build`    | `ci/gates/*.py`    |

If a rule would catch the same failure from a `git push` as from a
Claude edit, it's a gate. If it needs to see which tool is about to
run, write a hook.

## Adding a gate

1. Create `ci/gates/<name>.py` exposing `def run() -> int`.
2. Insert `<name>` into `ci.py::GATE_ORDER` at the right position.
3. Add a row to `tests/test_gates.py` covering the happy path.
4. The workflow picks it up automatically because `./ci.py all` walks
   `GATE_ORDER`; no `.github/workflows/ci.yml` edit needed unless the
   gate has a different platform affinity.
