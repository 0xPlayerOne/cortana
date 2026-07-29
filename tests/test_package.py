from importlib.metadata import version

from cortana import __version__


def test_runtime_version_matches_package_metadata() -> None:
    assert __version__ == version("cortana-brain")
