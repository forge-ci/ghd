import pytest

from ghd import _native
from ghd.bindings import version_banner


def test_version_banner_returns_string() -> None:
    assert isinstance(version_banner(), str)


def test_native_binding_uses_shared_repository_validation() -> None:
    with pytest.raises(OSError, match="repository"):
        _native.pr_view("host/owner", "repo/extra", 1, ["state"])
    with pytest.raises(OSError, match="different hosts"):
        _native.pr_view("first.example/owner", "repo", 1, ["state"], "second.example")
