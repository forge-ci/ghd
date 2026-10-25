"""`ci/auto_release.py`'s decision table: when a main-branch event becomes a
release candidate, a publish, or nothing."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from ci.auto_release import decide  # pylint: disable=wrong-import-position

SHA = "a" * 40
MAIN = "refs/heads/main"


def cargo(version: str) -> str:
    return f'[workspace]\nmembers = []\n[workspace.package]\nversion = "{version}"\n'


def pyproject(version: str) -> str:
    return f'[project]\nname = "ghd"\nversion = "{version}"\n'


def repo(head: str, parent: str | None, py: str | None = None):
    files = {
        (SHA, "Cargo.toml"): cargo(head),
        (SHA, "pyproject.toml"): pyproject(py or head),
    }
    if parent is not None:
        files[(f"{SHA}^", "Cargo.toml")] = cargo(parent)
    return lambda sha, path: files.get((sha, path))


def green_run(
    *,
    path=".github/workflows/ci.yml",
    event="workflow_dispatch",
    title=f"release {SHA}",
    verify="success",
):
    return lambda run_id: (
        {"path": path, "event": event, "display_title": title},
        [{"name": "release-verify", "conclusion": verify}],
    )


def run(*, event="push", ref=MAIN, show, released=(), fetch_run=None, run_id="42"):  # pylint: disable=too-many-arguments
    return decide(
        event=event,
        ref=ref,
        head_sha=SHA,
        candidate_sha=SHA if event == "workflow_dispatch" else "",
        run_id=run_id if event == "workflow_dispatch" else "",
        show=show,
        tag_exists=lambda tag: tag in released,
        fetch_run=fetch_run or green_run(),
    )


def test_version_bump_push_is_a_candidate() -> None:
    d = run(show=repo("0.2.0", "0.1.0"))
    assert (d.action, d.tag, d.sha) == ("candidate", "v0.2.0", SHA)


def test_push_without_a_bump_does_nothing() -> None:
    assert run(show=repo("0.1.0", "0.1.0")).action == "none"


def test_handoff_from_a_green_release_run_publishes() -> None:
    d = run(event="workflow_dispatch", show=repo("0.2.0", "0.1.0"))
    assert (d.action, d.tag, d.run_id) == ("publish", "v0.2.0", "42")


def test_already_released_tag_is_never_republished() -> None:
    d = run(event="workflow_dispatch", show=repo("0.2.0", "0.1.0"), released={"v0.2.0"})
    assert d.action == "none"


@pytest.mark.parametrize(
    "fetch_run, problem",
    [
        (green_run(verify="failure"), "release-verify did not succeed"),
        (green_run(event="push"), "not workflow_dispatch"),
        (green_run(title="release " + "b" * 40), "titled"),
        (green_run(path=".github/workflows/other.yml"), "not .github/workflows/ci.yml"),
    ],
)
def test_publish_refuses_a_run_that_did_not_verify_this_sha(
    fetch_run, problem: str
) -> None:
    with pytest.raises(ValueError, match=problem):
        run(event="workflow_dispatch", show=repo("0.2.0", "0.1.0"), fetch_run=fetch_run)


def test_non_main_ref_never_releases() -> None:
    assert run(ref="refs/heads/feature", show=repo("0.2.0", "0.1.0")).action == "none"


def test_name_reservation_is_never_released() -> None:
    assert run(show=repo("0.0.0", "0.0.0")).action == "none"


def test_initial_commit_is_not_a_bump() -> None:
    assert run(show=repo("0.1.0", None)).action == "none"


def test_pyproject_version_drift_fails_closed() -> None:
    with pytest.raises(ValueError, match="pyproject.toml version"):
        run(show=repo("0.2.0", "0.1.0", py="0.1.9"))


def test_dispatch_needs_a_numeric_run_id() -> None:
    with pytest.raises(ValueError, match="full_ci_run_id"):
        run(event="workflow_dispatch", show=repo("0.2.0", "0.1.0"), run_id="latest")
