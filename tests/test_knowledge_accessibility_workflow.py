from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/knowledge-accessibility.yml"


def test_standalone_renderer_workflow_exports_provenance_and_large_fixture() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")

    for marker in (
        "CORTANA_KNOWLEDGE_TARGET=x86_64-unknown-linux-gnu",
        "CORTANA_KNOWLEDGE_VERSION=",
        "tauri.conf.json",
        "CORTANA_KNOWLEDGE_REVISION=",
        "GITHUB_SHA",
        "CORTANA_KNOWLEDGE_INSTALLATION_TYPE=prospective-source-renderer",
        "CORTANA_KNOWLEDGE_RUN_LARGE=true",
    ):
        assert marker in workflow
