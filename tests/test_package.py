from cortana import __version__


def test_version_matches_initial_release() -> None:
    assert __version__ == "0.1.0"
