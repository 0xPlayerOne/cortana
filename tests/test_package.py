import json
from importlib.metadata import version
from pathlib import Path

import tomllib

from cortana import __version__

ROOT = Path(__file__).resolve().parents[1]


def test_runtime_version_matches_package_metadata() -> None:
    assert __version__ == version("cortana-brain")


def test_release_manifest_has_one_shared_application_version() -> None:
    manifest = json.loads((ROOT / ".release-please-manifest.json").read_text())
    release_config = json.loads((ROOT / "release-please-config.json").read_text())
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    python = tomllib.loads((ROOT / "pyproject.toml").read_text())
    web = json.loads((ROOT / "apps/web/package.json").read_text())

    assert list(manifest) == ["."]
    assert list(release_config["packages"]) == ["."]
    assert {
        manifest["."],
        cargo["package"]["version"],
        python["project"]["version"],
        web["version"],
        __version__,
    } == {manifest["."]}
