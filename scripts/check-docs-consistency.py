#!/usr/bin/env python3
"""Check Cortana documentation ownership and planning-source consistency."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

RELEASE_HISTORY = ROOT / "docs" / "releases.md"

PLANNING_AUTHORITY_FILES = [
    ROOT / "README.md",
    ROOT / "docs" / "README.md",
    ROOT / "docs" / "planning.md",
    ROOT / "docs" / "project-goal.md",
    ROOT / "docs" / "source-rollout.md",
    ROOT / "docs" / "desktop-ux-audit.md",
    ROOT / "docs" / "evaluation.md",
    ROOT / "docs" / "operations.md",
]

MILESTONE_LINK = "https://github.com/0xPlayerOne/cortana/milestones"
ISSUE_LINK = "https://github.com/0xPlayerOne/cortana/issues"

CURRENT_RELEASE_PATTERN = re.compile(
    r"(?m)^## Current release: v(?P<version>\d+\.\d+\.\d+)\s*$"
)

FORBIDDEN_PLANNING_HEADINGS = [
    re.compile(r"(?mi)^##+\s+Current status\s*$"),
    re.compile(r"(?mi)^##+\s+Current operator state\b.*$"),
    re.compile(r"(?mi)^##+\s+Remaining production milestones\s*$"),
    re.compile(r"(?mi)^##+\s+Roadmap-level product sequence\s*$"),
    re.compile(r"(?mi)^##+\s+Production blockers\b.*$"),
    re.compile(r"(?mi)^##+\s+Open (product|technical) decisions\s*$"),
]


def read(path: Path) -> str:
    if not path.exists():
        raise AssertionError(f"missing documentation file: {path.relative_to(ROOT)}")
    return path.read_text(encoding="utf-8")


def current_release_version(text: str) -> str:
    matches = list(CURRENT_RELEASE_PATTERN.finditer(text))
    if len(matches) != 1:
        raise AssertionError(
            "docs/releases.md must contain exactly one "
            "'## Current release: vX.Y.Z' heading"
        )
    return matches[0].group("version")


def display_path(path: Path) -> Path:
    try:
        return path.relative_to(ROOT)
    except ValueError:
        return path


def check_planning_authority(path: Path, text: str) -> list[str]:
    errors: list[str] = []
    rel = display_path(path)

    if MILESTONE_LINK not in text:
        errors.append(f"{rel}: missing canonical GitHub milestones link")
    if ISSUE_LINK not in text:
        errors.append(f"{rel}: missing canonical GitHub issues link")

    for pattern in FORBIDDEN_PLANNING_HEADINGS:
        match = pattern.search(text)
        if match:
            errors.append(
                f"{rel}: contains forbidden parallel-planning heading "
                f"{match.group(0)!r}"
            )

    return errors


def main() -> int:
    errors: list[str] = []

    try:
        release_text = read(RELEASE_HISTORY)
        current_release_version(release_text)
    except AssertionError as exc:
        errors.append(str(exc))

    for path in PLANNING_AUTHORITY_FILES:
        try:
            text = read(path)
        except AssertionError as exc:
            errors.append(str(exc))
            continue
        errors.extend(check_planning_authority(path, text))

    if errors:
        print("Documentation consistency check failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print("Documentation consistency check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
