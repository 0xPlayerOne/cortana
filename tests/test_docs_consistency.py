from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check-docs-consistency.py"

spec = importlib.util.spec_from_file_location("check_docs_consistency", SCRIPT)
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def test_current_release_heading_is_unambiguous() -> None:
    assert module.current_release_version("## Current release: v1.2.3\n") == "1.2.3"

    with pytest.raises(AssertionError):
        module.current_release_version("")

    with pytest.raises(AssertionError):
        module.current_release_version("## Current release: v1.2.3\n## Current release: v1.2.4\n")


def test_planning_authority_requires_both_links(tmp_path: Path) -> None:
    path = tmp_path / "doc.md"

    errors = module.check_planning_authority(path, "# Doc\n")
    assert any("milestones link" in error for error in errors)
    assert any("issues link" in error for error in errors)

    text = f"[Milestones]({module.MILESTONE_LINK})\n[Issues]({module.ISSUE_LINK})\n"
    assert module.check_planning_authority(path, text) == []


@pytest.mark.parametrize(
    "heading",
    [
        "## Current status",
        "## Current operator state (today)",
        "## Remaining Production Milestones",
        "## Roadmap-Level Product Sequence",
        "## Production blockers before launch",
        "## Open Product Decisions",
        "## Open Technical Decisions",
    ],
)
def test_parallel_planning_headings_are_rejected(tmp_path: Path, heading: str) -> None:
    path = tmp_path / "doc.md"
    text = f"[Milestones]({module.MILESTONE_LINK})\n[Issues]({module.ISSUE_LINK})\n{heading}\n"
    errors = module.check_planning_authority(path, text)
    assert any("forbidden parallel-planning heading" in error for error in errors)


def test_repository_documentation_passes() -> None:
    assert module.main() == 0


def test_release_history_does_not_label_historical_claims_as_current() -> None:
    text = (ROOT / "docs" / "releases.md").read_text(encoding="utf-8")

    assert "current source and package claims belong to" not in text


def test_current_release_matches_all_versioned_project_manifests() -> None:
    release = module.current_release_version(
        (ROOT / "docs" / "releases.md").read_text(encoding="utf-8")
    )

    versions = module.project_release_versions()

    assert versions
    assert set(versions.values()) == {release}


def test_repository_check_rejects_a_stale_project_manifest(monkeypatch: pytest.MonkeyPatch) -> None:
    versions = module.project_release_versions()
    versions["Web app"] = "0.56.2"
    monkeypatch.setattr(module, "project_release_versions", lambda: versions)

    assert module.main() == 1


def test_active_documentation_rejects_stale_release_references(tmp_path: Path) -> None:
    current = tmp_path / "current.md"
    stale = tmp_path / "stale.md"
    current.write_text("Current package: v0.56.3\n", encoding="utf-8")
    stale.write_text("Old package: v0.56.2\n", encoding="utf-8")

    errors = module.check_active_release_references(
        "0.56.3",
        [current, stale],
    )

    assert len(errors) == 1
    assert "stale.md" in errors[0]
    assert "v0.56.2" in errors[0]


def test_release_history_is_not_part_of_active_release_reference_checks() -> None:
    assert module.RELEASE_HISTORY not in module.ACTIVE_DOCUMENTATION_FILES


def test_current_release_section_rejects_stale_release_references() -> None:
    text = (
        "## Current release: v0.56.3\n"
        "The current patch line includes v0.56.1 through v0.56.3.\n"
        "A stale current claim names v0.53.5.\n"
        "### Historical context\n"
        "This heading may mention v0.39.0.\n"
        "## v0.53.5 release notes (historical)\n"
    )

    errors = module.check_current_release_references(text, "0.56.3")

    assert len(errors) == 1
    assert "v0.53.5" in errors[0]


def test_current_release_section_allows_same_patch_line_references() -> None:
    text = "## Current release: v0.56.3\nThe v0.56.1 and v0.56.2 patch releases are included.\n"

    assert module.check_current_release_references(text, "0.56.3") == []
