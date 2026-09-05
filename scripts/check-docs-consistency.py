#!/usr/bin/env python3
"""Check Cortana documentation ownership and planning-source consistency."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import tomllib

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

ACTIVE_DOCUMENTATION_FILES = [
    ROOT / "README.md",
    *sorted(path for path in (ROOT / "docs").glob("*.md") if path != RELEASE_HISTORY),
    ROOT / "apps" / "desktop" / "README.md",
]
MILESTONE_LINK = "https://github.com/adea-ai/cortana/milestones"
ISSUE_LINK = "https://github.com/adea-ai/cortana/issues"

CURRENT_RELEASE_PATTERN = re.compile(r"(?m)^## Current release: v(?P<version>\d+\.\d+\.\d+)\s*$")
VERSION_REFERENCE_PATTERN = re.compile(r"\bv(?P<version>\d+\.\d+\.\d+)\b")

VERSIONED_PROJECT_MANIFESTS = (
    ("Rust core", Path("Cargo.toml"), "toml-package"),
    ("Desktop Rust crate", Path("apps/desktop/src-tauri/Cargo.toml"), "toml-package"),
    ("Connector", Path("pyproject.toml"), "toml-project"),
    ("Web app", Path("apps/web/package.json"), "json"),
    ("Desktop app", Path("apps/desktop/package.json"), "json"),
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
            "docs/releases.md must contain exactly one '## Current release: vX.Y.Z' heading"
        )
    return matches[0].group("version")


def project_release_versions(root: Path = ROOT) -> dict[str, str]:
    versions: dict[str, str] = {}
    for label, relative_path, format_name in VERSIONED_PROJECT_MANIFESTS:
        path = root / relative_path
        if format_name == "json":
            value = json.loads(path.read_text(encoding="utf-8")).get("version")
        else:
            document = tomllib.loads(path.read_text(encoding="utf-8"))
            section = "package" if format_name == "toml-package" else "project"
            value = document.get(section, {}).get("version")
        if not isinstance(value, str) or not re.fullmatch(r"\d+\.\d+\.\d+", value):
            raise AssertionError(f"{label} manifest has no semantic release version")
        versions[label] = value
    return versions


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
            errors.append(f"{rel}: contains forbidden parallel-planning heading {match.group(0)!r}")

    return errors


def check_active_release_references(
    release_version: str,
    paths: list[Path] | tuple[Path, ...] = ACTIVE_DOCUMENTATION_FILES,
) -> list[str]:
    errors: list[str] = []

    for path in paths:
        text = read(path)
        for match in VERSION_REFERENCE_PATTERN.finditer(text):
            version = match.group("version")
            if version == release_version:
                continue
            line = text.count("\n", 0, match.start()) + 1
            errors.append(
                f"{display_path(path)}:{line}: active documentation references stale release "
                f"v{version}; current release is v{release_version}"
            )

    return errors


def check_current_release_references(text: str, release_version: str) -> list[str]:
    heading = CURRENT_RELEASE_PATTERN.search(text)
    if heading is None:
        return []

    body_start = heading.end()
    next_heading = re.search(r"(?m)^#{2,}\s", text[body_start:])
    body_end = body_start + next_heading.start() if next_heading else len(text)
    current_body = text[body_start:body_end]
    release_line = ".".join(release_version.split(".")[:2]) + "."
    errors: list[str] = []

    for match in VERSION_REFERENCE_PATTERN.finditer(current_body):
        version = match.group("version")
        if version == release_version or version.startswith(release_line):
            continue
        absolute_offset = body_start + match.start()
        line = text.count("\n", 0, absolute_offset) + 1
        errors.append(
            f"{display_path(RELEASE_HISTORY)}:{line}: current release section references stale "
            f"release v{version}; current release is v{release_version}"
        )

    return errors


def main() -> int:
    errors: list[str] = []
    release_version: str | None = None

    try:
        release_text = read(RELEASE_HISTORY)
        release_version = current_release_version(release_text)
    except AssertionError as exc:
        errors.append(str(exc))

    if release_version is not None:
        try:
            errors.extend(check_current_release_references(release_text, release_version))
            errors.extend(check_active_release_references(release_version))
        except AssertionError as exc:
            errors.append(str(exc))

        try:
            versions = project_release_versions()
            mismatches = [
                f"{label}={version}"
                for label, version in versions.items()
                if version != release_version
            ]
            if mismatches:
                errors.append(
                    "docs/releases.md current release does not match project manifests: "
                    + ", ".join(mismatches)
                )
        except (AssertionError, OSError, json.JSONDecodeError, tomllib.TOMLDecodeError) as exc:
            errors.append(f"project release manifest check failed: {exc}")

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
