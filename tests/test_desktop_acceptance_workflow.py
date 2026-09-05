from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/desktop-acceptance.yml"


def test_acceptance_matrix_keeps_target_artifacts_separate() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")

    assert "pattern: desktop-acceptance-*" in workflow
    assert "merge-multiple: false" in workflow
    assert "merge-multiple: true" not in workflow


def test_acceptance_workflow_runs_each_strict_evidence_lane() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")

    for command in (
        "desktop-package-acceptance.mjs",
        "desktop-install-acceptance.mjs",
        "desktop-control-plane-acceptance.mjs",
        "desktop-service-status-acceptance.mjs",
        "desktop-source-authorization-acceptance.mjs",
        "desktop-host-launch.mjs",
        "desktop-macos-lifecycle-acceptance.mjs",
        "knowledge-accessibility-acceptance.mjs",
        "desktop-acceptance-matrix.mjs",
    ):
        assert command in workflow


def test_acceptance_workflow_downloads_updater_manifest_for_each_target() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")

    assert workflow.count("latest.json") >= 3
    assert "latest.json'" in workflow


def test_acceptance_workflow_gates_the_macos_native_lifecycle_lane() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")
    start = workflow.index("Run supplemental macOS native lifecycle acceptance")
    end = workflow.index("Run packaged desktop host launch (Windows)", start)

    assert "continue-on-error" not in workflow[start:end]


def test_acceptance_workflow_uses_the_release_compatible_renderer_fixture() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")

    assert workflow.count("CORTANA_KNOWLEDGE_RUN_LARGE=false") == 1
    assert workflow.count("$env:CORTANA_KNOWLEDGE_RUN_LARGE = 'false'") == 1
    assert "CORTANA_KNOWLEDGE_RUN_LARGE=true" not in workflow


def test_acceptance_workflow_labels_published_renderer_provenance() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")

    assert workflow.count("CORTANA_KNOWLEDGE_INSTALLATION_TYPE=published-package-renderer") == 1
    assert (
        workflow.count("$env:CORTANA_KNOWLEDGE_INSTALLATION_TYPE = 'published-package-renderer'")
        == 1
    )
