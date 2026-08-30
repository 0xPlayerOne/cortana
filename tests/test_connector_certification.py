from __future__ import annotations

import dataclasses
import json

import pytest

from cortana.connectors.__main__ import main
from cortana.connectors.certification import (
    BUILTIN_MANIFESTS,
    certify_builtin,
    fixture_run,
    stable_document_id,
    validate_run,
)


@pytest.mark.parametrize("connector_id", ["slack", "discord", "external-reference"])
def test_synthetic_connector_certification_passes_without_personal_accounts(
    connector_id: str,
) -> None:
    report = certify_builtin(connector_id)
    assert report.approved
    assert report.fixture_only
    assert report.checks["disabled_by_default"]
    assert report.checks["failures_do_not_reconcile"]


def test_partial_cancelled_revoked_and_rate_limited_runs_never_reconcile() -> None:
    manifest = BUILTIN_MANIFESTS["slack"]
    for status in ("partial", "cancelled", "revoked", "rate_limited"):
        assert not validate_run(manifest, fixture_run("slack", status)).reconcile_allowed


def test_certification_rejects_duplicate_ids_secrets_and_incompatible_versions() -> None:
    manifest = BUILTIN_MANIFESTS["external-reference"]
    run = fixture_run("external-reference", "succeeded")
    duplicate = dataclasses.replace(
        run,
        documents=(run.documents[0], run.documents[0]),
        progress_documents=2,
        progress_bytes=2 * run.progress_bytes,
    )
    with pytest.raises(RuntimeError, match="duplicate"):
        validate_run(manifest, duplicate)
    secret = dataclasses.replace(run.documents[0], content="Authorization: Bearer fixture-secret")
    secret_run = dataclasses.replace(
        run,
        documents=(secret,),
        progress_bytes=len(secret.as_json().encode()) + 1,
    )
    with pytest.raises(RuntimeError, match="credential"):
        validate_run(manifest, secret_run)
    incompatible = dataclasses.replace(manifest, contract_version="future")
    with pytest.raises(RuntimeError, match="incompatible"):
        validate_run(incompatible, run)


def test_certification_rejects_invalid_fingerprints_cursors_paths_and_progress() -> None:
    manifest = BUILTIN_MANIFESTS["external-reference"]
    run = fixture_run("external-reference", "succeeded")
    with pytest.raises(RuntimeError, match="fingerprint"):
        validate_run(manifest, dataclasses.replace(run, configuration_fingerprint="invalid"))
    with pytest.raises(RuntimeError, match="cursor"):
        validate_run(manifest, dataclasses.replace(run, cursor="bad\ncursor"))
    path_document = dataclasses.replace(run.documents[0], uri="file:///Users/person/private.txt")
    with pytest.raises(RuntimeError, match="private absolute path"):
        validate_run(
            manifest,
            dataclasses.replace(
                run,
                documents=(path_document,),
                progress_bytes=len(path_document.as_json().encode()) + 1,
            ),
        )
    with pytest.raises(RuntimeError, match="negative"):
        validate_run(manifest, dataclasses.replace(run, progress_documents=-1))


def test_source_identity_is_stable_and_disable_does_not_delete() -> None:
    run = fixture_run("discord", "succeeded")
    identity = stable_document_id(run.documents[0])
    assert identity == stable_document_id(run.documents[0])
    assert not BUILTIN_MANIFESTS["discord"].enabled_by_default
    assert len(run.documents) == 1, "validation has no deletion side effect"


def test_certification_cli_emits_machine_readable_report(
    capsys: pytest.CaptureFixture[str],
) -> None:
    assert main(["certify", "slack"]) == 0
    report = json.loads(capsys.readouterr().out)
    assert report["approved"] is True
    assert report["connector_id"] == "slack"
