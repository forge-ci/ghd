from ghd.bindings import version_banner


def test_version_banner_returns_string() -> None:
    assert isinstance(version_banner(), str)
