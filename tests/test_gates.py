"""Contract test for the gates infrastructure.

Asserts that every gate registered in `ci.py::GATE_ORDER`:

  1. resolves to a module under `ci/gates/`,
  2. exposes a zero-arg `run()`,
  3. has `run()` annotated to return an int.

This does NOT execute the gates — that's what `./ci.py all` is for. The
contract test exists so future gate additions can't accidentally ship a
broken signature; a developer who registers a new gate but forgets the
`def run() -> int` shape sees the failure here instead of mid-CI.
"""

from __future__ import annotations

import importlib
import importlib.util
import inspect
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT))


def _load_gate_order() -> list[str]:
    # Import the dispatcher's GATE_ORDER without executing the script
    # body (it has argparse / SystemExit in main()).
    spec = importlib.util.spec_from_file_location("_ci_dispatcher", REPO_ROOT / "ci.py")
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return list(mod.GATE_ORDER)


GATE_NAMES = _load_gate_order()


@pytest.mark.parametrize("name", GATE_NAMES)
def test_gate_module_is_importable(name: str) -> None:
    mod = importlib.import_module(f"ci.gates.{name}")
    assert mod is not None


@pytest.mark.parametrize("name", GATE_NAMES)
def test_gate_exposes_run(name: str) -> None:
    mod = importlib.import_module(f"ci.gates.{name}")
    assert hasattr(mod, "run"), f"ci.gates.{name} must expose `def run()`"
    fn = mod.run
    assert callable(fn), f"ci.gates.{name}.run must be callable"


@pytest.mark.parametrize("name", GATE_NAMES)
def test_gate_run_signature_is_zero_arg(name: str) -> None:
    mod = importlib.import_module(f"ci.gates.{name}")
    sig = inspect.signature(mod.run)
    required = [
        p
        for p in sig.parameters.values()
        if p.default is inspect.Parameter.empty
        and p.kind
        not in (inspect.Parameter.VAR_POSITIONAL, inspect.Parameter.VAR_KEYWORD)
    ]
    assert not required, (
        f"ci.gates.{name}.run must take no required arguments (found: {[p.name for p in required]})"
    )


@pytest.mark.parametrize("name", GATE_NAMES)
def test_gate_run_returns_int_annotation(name: str) -> None:
    mod = importlib.import_module(f"ci.gates.{name}")
    sig = inspect.signature(mod.run)
    ann = sig.return_annotation
    # Accept either `int` directly or the string "int" (PEP 563 future-annotations).
    assert ann in (int, "int"), (
        f"ci.gates.{name}.run should be annotated `-> int` (got {ann!r})"
    )


def test_loc_excludes_ci_lint_but_checks_repository_source(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Precheck downloads third-party code; it must not change our LOC verdict."""
    loc = importlib.import_module("ci.gates.loc")

    monkeypatch.setattr(loc, "ROOT", tmp_path)
    checkout = tmp_path / ".ci-lint"
    checkout.mkdir()
    (checkout / "external.py").write_text("# external\n" * (loc.ERROR + 1))
    source = tmp_path / "source.py"
    source.write_text("# source\n")
    assert loc.run() == 0
    source.write_text("# source\n" * (loc.ERROR + 1))
    assert loc.run() == 1
