from __future__ import annotations

import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/check-docs-consistency.py"


def _load_checker():
    spec = importlib.util.spec_from_file_location("cortana_docs_consistency", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_current_release_heading_is_unambiguous() -> None:
    checker = _load_checker()
    assert checker.current_version("## Current release: v1.2.3\n") == "1.2.3"

    for text in (
        "# Releases\n",
        "## Current release: v1.2.3\n## Current release: v1.2.4\n",
    ):
        try:
            checker.current_version(text)
        except ValueError:
            pass
        else:
            raise AssertionError("ambiguous release heading should fail")


def test_current_section_does_not_accept_archived_marker() -> None:
    checker = _load_checker()
    text = "## Current status\nThis is v1.2.3.\n## Archive\nThis is v9.9.9.\n"
    body = checker.section_body(text, "## Current status", "## Archive")
    assert body == "\nThis is v1.2.3.\n"
    assert "v9.9.9" not in body


def test_current_documentation_matches_release_boundary() -> None:
    checker = _load_checker()
    assert checker.check_docs(ROOT) == []
