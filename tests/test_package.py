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
    uv_lock_text = (ROOT / "uv.lock").read_text()
    uv_lock = tomllib.loads(uv_lock_text)
    web = json.loads((ROOT / "apps/web/package.json").read_text())
    desktop = json.loads((ROOT / "apps/desktop/package.json").read_text())
    desktop_cargo = tomllib.loads((ROOT / "apps/desktop/src-tauri/Cargo.toml").read_text())
    desktop_lock_text = (ROOT / "apps/desktop/src-tauri/Cargo.lock").read_text()
    desktop_lock = tomllib.loads(desktop_lock_text)
    desktop_config = json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text())
    desktop_lock_version = next(
        package["version"]
        for package in desktop_lock["package"]
        if package["name"] == "cortana-desktop"
    )
    uv_lock_version = next(
        package["version"] for package in uv_lock["package"] if package["name"] == "cortana-brain"
    )

    assert list(manifest) == ["."]
    assert list(release_config["packages"]) == ["."]
    assert {
        manifest["."],
        cargo["package"]["version"],
        python["project"]["version"],
        web["version"],
        desktop["version"],
        desktop_cargo["package"]["version"],
        desktop_lock_version,
        uv_lock_version,
        desktop_config["version"],
        __version__,
    } == {manifest["."]}
    assert (
        f'name = "cortana-brain"\nversion = "{manifest["."]}" # x-release-please-version'
        in uv_lock_text
    )
    assert (
        f'name = "cortana-desktop"\nversion = "{manifest["."]}" # x-release-please-version'
        in desktop_lock_text
    )
    for script_name in (
        "build",
        "build:release",
        "bundle:mac",
        "check",
        "clippy",
        "dev",
        "test",
        "test:native",
    ):
        assert "../../scripts/run-desktop-command.mjs" in desktop["scripts"][script_name]
