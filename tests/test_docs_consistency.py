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
        module.current_release_version(
            "## Current release: v1.2.3\n## Current release: v1.2.4\n"
        )


def test_planning_authority_requires_both_links(tmp_path: Path) -> None:
    path = tmp_path / "doc.md"

    errors = module.check_planning_authority(path, "# Doc\n")
    assert any("milestones link" in error for error in errors)
    assert any("issues link" in error for error in errors)

    text = (
        f"[Milestones]({module.MILESTONE_LINK})\n"
        f"[Issues]({module.ISSUE_LINK})\n"
    )
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
def test_parallel_planning_headings_are_rejected(
    tmp_path: Path, heading: str
) -> None:
    path = tmp_path / "doc.md"
    text = (
        f"[Milestones]({module.MILESTONE_LINK})\n"
        f"[Issues]({module.ISSUE_LINK})\n"
        f"{heading}\n"
    )
    errors = module.check_planning_authority(path, text)
    assert any("forbidden parallel-planning heading" in error for error in errors)


def test_repository_documentation_passes() -> None:
    assert module.main() == 0
