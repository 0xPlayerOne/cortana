#!/usr/bin/env python3
"""Verify that user-facing documentation shares one current release boundary.

Release Please owns version bumps and changelog generation, while current
release evidence is maintained in ``docs/releases.md``. This check deliberately
does not compare that evidence to package manifests: a release PR may bump
manifests before the evidence is refreshed. It does ensure that no user or
operator entry point silently lags the documented release boundary.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CURRENT_RELEASE_RE = re.compile(r"^## Current release:\s*v(\d+\.\d+\.\d+)\s*$", re.MULTILINE)

# These are the user and operator entry points. Each explicit end heading
# bounds the current-release section, so an old version mentioned only in an
# archive cannot accidentally satisfy the check.
CURRENT_SECTIONS = {
    Path("README.md"): ("## Download the latest release", "## Quick start"),
    Path("docs/README.md"): ("## Current status", "## Start here"),
    Path("docs/getting-started.md"): (
        "## The shortest path to a first result",
        "## What you get",
    ),
    Path("docs/project-goal.md"): ("## Release boundary", "## Where to start"),
    Path("docs/evaluation.md"): (
        "### Current release boundary",
        "## Bounded disposable load benchmark",
    ),
    Path("docs/desktop-ux-audit.md"): ("## Current release evidence", "## Requirement matrix"),
    Path("docs/operations.md"): ("## Release verification", "## macOS launchd"),
    Path("docs/source-rollout.md"): ("## Current operator state", "## Per-source rollout matrix"),
    Path("docs/ingestion.md"): ("## Current release boundary", "## Configure and run"),
    Path("docs/integrations.md"): ("## Current release boundary", "## Install the portable skill"),
    Path("docs/query.md"): ("## Current release boundary", "## Canonical document browser"),
    Path("docs/memory.md"): ("## Current release boundary", "## Memory model"),
}


def current_version(releases_text: str) -> str:
    matches = CURRENT_RELEASE_RE.findall(releases_text)
    if len(matches) != 1:
        raise ValueError(
            "docs/releases.md must contain exactly one '## Current release: vX.Y.Z' heading"
        )
    return matches[0]


def section_body(text: str, start: str, end: str) -> str:
    """Return the bounded Markdown body between two explicit headings."""
    start_match = re.search(rf"^{re.escape(start)}.*$", text, re.MULTILINE)
    if start_match is None:
        raise ValueError(f"missing documentation section heading: {start}")
    end_match = re.search(rf"^{re.escape(end)}.*$", text[start_match.end() :], re.MULTILINE)
    if end_match is None:
        raise ValueError(f"missing documentation section boundary: {end}")
    return text[start_match.end() : start_match.end() + end_match.start()]


def check_docs(root: Path = ROOT) -> list[str]:
    releases_path = root / "docs/releases.md"
    try:
        releases_text = releases_path.read_text(encoding="utf-8")
    except OSError as error:
        return [f"cannot read {releases_path}: {error}"]

    try:
        version = current_version(releases_text)
    except ValueError as error:
        return [str(error)]

    marker = f"v{version}"
    errors: list[str] = []
    for relative_path, (start, end) in CURRENT_SECTIONS.items():
        path = root / relative_path
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{relative_path}: cannot read file: {error}")
            continue
        try:
            body = section_body(text, start, end)
        except ValueError as error:
            errors.append(f"{relative_path}: {error}")
            continue
        if marker not in body:
            errors.append(
                f"{relative_path}: section {start!r} missing current release marker {marker}"
            )
    return errors


def main() -> int:
    errors = check_docs()
    if errors:
        print("Documentation consistency check failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    version = current_version((ROOT / "docs/releases.md").read_text(encoding="utf-8"))
    print(f"documentation consistency check passed for v{version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
