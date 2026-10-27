# Architecture


Default `python3 ci/local.py act` requires a clean committed tree, uses the
shared gate's execution or fresh-result reuse, and amends HEAD on success.
See the [local-loop contract](../CLAUDE.md#ci).
Explicit title/job selections remain diagnostics; native and release coverage
remain required.

## Goal

Ship one Python wheel that exposes:

- a Python import surface backed by `PyO3` (`ghd._native`)
- a command surface backed by a compiled Rust executable (`ghd-cli`,
  bundled into the wheel by the soldr PEP 517 backend at
  `ghd-<ver>.data/scripts/` — pip extracts it
  straight into the venv's `Scripts/`/`bin/` on install, no Python
  launcher; see #7 for the Windows `os.execv` race that motivated this)

A consumer installs a single distribution and gets both deliverables.
A maintainer maintains one workspace and (eventually, once a later
round of [zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6)
lands it) one generated CI matrix to feed both.

## CI Architecture

This template implements the canonical Rust+Python gate/hook shape
from [`zackees/zccache#835`](https://github.com/zackees/zccache/issues/835),
and is the fleet reference repo for the `rust-pypi-app` profile defined
in [`zackees/ci.yml#6`](https://github.com/zackees/ci.yml/issues/6).
Load-bearing pieces:

1. **`ci.toml`** (repo root). The CI contract: declared platforms, the
   Rust public/private crate split and test-binary budget, Python
   packaging shape, suites, flows, tags, cache families. Does not
   generate YAML — a future `ci-lint` (built in `zackees/ci.yml`)
   checks the repo against it and computes each run's plan.
2. **`./ci.py`.** PEP 723 dispatcher, directly executable via its
   `#!/usr/bin/env -S uv run --no-project --script` shebang. Every
   gate invocation goes through here so the `--no-project --script`
   flag combo (which suppresses the soldr-backend auto-build trap)
   lives in one place.
3. **`ci/gates/`.** Workspace-state checks. Each file exposes
   `def run() -> int`. Canonical ordering in `ci.py::GATE_ORDER`. Every
   Rust/wheel command goes through `soldr` — never bare `cargo`/`maturin`.
4. **`ci/hooks/`.** Agent-intent guards wired through
   `.claude/settings.json`. Only fires during Claude/Codex sessions.
5. **No `.github/workflows/` right now.** `zackees/ci.yml#6` round 1
   (this change) deleted the previous `ci.yml` — it queued a retired
   `macos-13` runner for 24h on every run and never passed. A later
   round adds `.github/workflows/ci.yml` (the only trigger-bearing
   workflow) + `ci-pre.yml` (workflow_call only), planned by
   `ci.toml`. Until then, `./ci.py all` locally is the equivalent.
6. **`action.yml` + `action/cleanup/action.yml`.** Composite action
   contract. Validated by `ci/gates/action_yaml.py` (structural) +
   `ci/gates/action_surface.py` (runtime binary surface match). Every
   `run:` step is one line calling a script under `action/` — no
   inline multi-line shell.

## Crate Responsibilities

### `crates/template` (public amalgam)

- **No logic of its own.** Every symbol is a `pub use` re-export from
  a crate under `crates/private/`.
- The only crate published to crates.io, and the only one
  `ghd-cli`/`ghd-py` depend on.
- Cargo features are `dep:` wiring only (`json = ["dep:ghd-daemon"]`)
  — never a behavioral `#[cfg(feature = ...)]` branch.

### `crates/private/ghd-core`

- pure Rust domain logic, `publish = false`
- no Python-specific concerns, no `[features]`
- reaches the OS only via `crate::platform::*` (aliased from
  `ghd-platform`)
- the unit of behavior consistency between the CLI and Python surfaces

### `crates/private/ghd-daemon`

- an optional capability (JSON encoding of the version banner) that is
  a whole crate, not a `#[cfg(feature)]` branch inside `ghd-core`
- no third-party dependencies — hand-rolled, deliberately

### `crates/private/ghd-platform`

- the host-platform facade: one `cfg_select!` selector (`lib.rs`), a
  cfg-free facility surface (`platform.rs` + `platform/**`), native
  code confined to `platforms/{windows,linux,macos}/**`
- the **only** crate allowed to select on host OS; every other crate
  reaches it through `pub(crate) use ghd_platform as platform;`

### `crates/ghd-cli`

- argument parsing (currently minimal), command execution
- stdout/stderr policy, exit-code policy
- depends on `template` (the amalgam) with the `json` ship feature —
  never on the private crates directly
- **subcommand/flag surface is part of the composite-action contract**
  — `ci/gates/action_surface.py` verifies `action.yml`'s scripts only
  reference surface actually present in `--help`

### `crates/ghd-py`

- `PyO3` module definitions (`#[pymodule]`, `#[pyfunction]`)
- Python-friendly value conversion
- thin translation layer over `template` (the amalgam) — no Rust test
  harness (`[lib] test = false, doctest = false`); tested from the
  Python side (`tests/test_bindings.py`)

### `src/ghd`

- package version and re-exports (`__init__.py`)
- Python wrapper around the extension (`bindings.py`), no pure-Python
  fallback for `_native`
- `platforms/` — host-platform facade (Python side), mirroring the
  Rust crate of the same purpose
- typing stub for the PyO3 surface (`_native.pyi`)

The `ghd-cli` binary is intentionally NOT under the package — it
ships via `bundle-bins` (see Goal above). A Python caller that wants to
invoke the CLI from code should do
`subprocess.run([shutil.which("ghd-cli"), ...])`, not import
anything from this package.

## Hook / Gate Split

| Concern                                | Home                       |
|----------------------------------------|----------------------------|
| Repo-state checks (every CI cycle)     | `ci/gates/*.py`            |
| Agent-intent checks (Claude sessions)  | `ci/hooks/*.py`            |
| File size budget (workspace-wide)      | `ci/gates/loc.py`          |
| File size budget (per-edit)            | `ci/hooks/loc_guard.py`    |
| README presence + size                 | `ci/hooks/readme_guard.py` |
| Bare cargo/maturin / unsafe `uv run` shape | `ci/hooks/tool_guard.py` |

## Non-goals

- duplicating core logic in Python
- embedding CLI-only behavior inside the `PyO3` module without an API
  use case
- maintaining separate CI infrastructure for the Python and Rust sides
- inline shell logic in any future workflow YAML, or in `action.yml`
- gates that hard-couple to the agent's session (those are hooks)
- a `#[cfg(feature = ...)]` branch inside a private crate's shared
  logic — features gate `dep:` wiring in `template` only

## Credential and host boundary

`ghd-core::identity` resolves the hostname and token in the caller. Daemon
endpoint discovery uses `identities-v1/<digest>` under the private state
root. The digest includes a versioned domain separator, normalized hostname
and credential. It contains no raw credential. The spawn environment pins
the resolved identity, preventing a stored-account change between discovery
and daemon startup from changing the cache owner.

A daemon serves one identity. Its cache and batch maps are private to that
process, and every loopback call carries the expected identity digest as
well as the session bearer token. Requests for another identity are rejected
before cache access. REST and GraphQL URLs derive from the selected deployment
(public GitHub, ghe.com or Enterprise Server). Both native Python clients and
CLI callers use the same core discovery/resolution path. Legacy unpartitioned
endpoint files are never reused.

## Write coherence and passthrough

`ghd-core::compat` launches real gh without a shell, inheriting the original
streams and arguments. Local auth/config commands and known reads are separate
from mutations and unknown commands. A write intent precedes subprocess
execution. A successful exit finishes it; failed/uncertain execution retains
it, while a definite spawn failure can safely finish without remote dispatch.
Exit codes propagate through a typed command-status error and the native CLI.

`ghd-core::coherence::Journal` stores a versioned protobuf generation and
outstanding write IDs under the private state root. File locks serialize
journal transitions across processes; file replacement publishes each record
atomically. The journal is shared across identities because an arbitrary gh
alias/extension can select a different host or credential than a superficial
argument scan would infer. It contains no credentials, repository names or
mutation payloads. Corrupt or unresolved records fail closed.

Daemon reads check the journal, synchronize their cache generation and check
again before returning bytes. A write clears PR and raw-query maps, and a
pre-write in-flight result cannot become authoritative after the generation
changes. The same fence is the coherence input for future persistent caches.
The journal currently proves process-crash custody; full durable snapshot
storage and filesystem crash validation belong to issue #11.

GraphQL classification uses an actual parser in ghd-core; operationName is
part of both the upstream envelope and the exact query cache key. The daemon
rejects invalid/ambiguous documents and subscriptions before execution.
Client reconnection is restricted to pre-dispatch situations; once connected,
a lost reply is an error rather than a second potentially mutating request.
