from __future__ import annotations

import importlib.util
from pathlib import Path

SCRIPT = Path(__file__).parents[1] / "scripts" / "check-release-version.py"
SPEC = importlib.util.spec_from_file_location("check_release_version", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_parse_version_reads_semver_from_manifest() -> None:
    assert MODULE.parse_version(
        '[dependencies]\nversion = "1.2"\n[package]\nversion = "0.32.0"\n',
        "Cargo.toml",
    ) == (
        0,
        32,
        0,
    )


def test_parse_version_accepts_a_tag_and_ignores_non_semver_tags() -> None:
    assert MODULE.parse_version("v0.32.0", "git tag") == (0, 32, 0)
    assert MODULE.parse_version('version = "1.0.0"', "Cargo.toml") == (1, 0, 0)


def test_release_branch_comparison_is_strictly_increasing() -> None:
    assert MODULE.parse_version('version = "0.32.1"', "Cargo.toml") > MODULE.parse_version(
        'version = "0.32.0"', "Cargo.toml"
    )
    assert not MODULE.parse_version('version = "0.31.1"', "Cargo.toml") > MODULE.parse_version(
        'version = "0.32.0"', "Cargo.toml"
    )
