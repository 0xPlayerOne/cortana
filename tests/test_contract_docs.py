"""Contract-source guardrails for the M2 durable specifications."""

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(relative: str) -> str:
    return (ROOT / relative).read_text(encoding="utf-8")


def test_m2_contract_documents_are_linked_and_versioned() -> None:
    docs = {
        "core": "docs/contracts/core-entities.md",
        "context": "docs/contracts/context-bundle.md",
        "memory": "docs/contracts/memory.md",
        "connectors": "docs/contracts/connectors.md",
        "identity": "docs/contracts/identity.md",
        "api": "docs/contracts/public-api.md",
        "security": "docs/security-model.md",
    }
    for name, relative in docs.items():
        text = read(relative)
        assert "contract" in text.lower(), name
        assert "cortana." in text, name
        assert len(text) > 600, name

    index = read("docs/README.md")
    for relative in docs.values():
        assert relative.removeprefix("docs/") in index, relative


def test_security_and_evaluation_contracts_keep_hard_safety_gates() -> None:
    security = read("docs/security-model.md")
    evaluation = read("docs/evaluation.md")
    for phrase in (
        "ACL",
        "credential",
        "reconciliation",
        "loopback",
        "prompt injection",
    ):
        assert phrase.lower() in security.lower(), phrase
    for phrase in (
        "ACL leak count",
        "invalid accepted citation",
        "approved-corpus",
        "deterministic CI",
    ):
        assert phrase.lower() in evaluation.lower(), phrase


def test_public_fixtures_are_transport_safe_and_normalized() -> None:
    bundle = json.loads(read("tests/fixtures/context-bundle-v1.json"))
    assert bundle["contract_version"] == "cortana.context.v1"
    assert bundle["context_bundle_id"].startswith("ctx_")
    assert len(bundle["canonical_digest"]) == 64
    assert len(bundle["privacy_scope_digest"]) == 64
    assert all(
        secret not in json.dumps(bundle).lower()
        for secret in ("password", "bearer", "api_key", "/users/")
    )

    rows = [json.loads(line) for line in read("tests/fixtures/connector-v1.jsonl").splitlines()]
    assert [row["source_id"] for row in rows] == ["doc-1", "doc-2"]
    assert all(row["source"] == "fixture" and row["acl"] == ["work"] for row in rows)


def test_provider_conformance_artifact_covers_profiles_failures_and_safety() -> None:
    artifact = json.loads(read("tests/fixtures/provider-conformance-v1.json"))
    assert artifact["fixture_version"] == "cortana.provider-fixtures.v1"
    assert artifact["provider_contract_version"] == "cortana.provider.v1"
    assert artifact["compatibility"]["deployment_profiles"] == [
        "local",
        "self_hosted_single_node",
    ]
    assert {case["name"] for case in artifact["semantic_cases"]} == {
        "evidence_only",
        "memory_enabled",
        "degraded",
        "stale",
        "over_budget",
        "empty",
        "contradictory",
        "cross_scope",
    }
    required_failures = {
        "missing_principal",
        "expired_principal",
        "revoked_principal",
        "wrong_project",
        "wrong_acl_scope",
        "local_restart",
        "self_hosted_restart",
        "broker_disconnect",
        "broker_reconnect",
        "duplicate_read",
        "duplicate_write",
        "ambiguous_write",
        "arbitrary_endpoint_attempt",
        "arbitrary_path_attempt",
    }
    assert required_failures <= set(artifact["failure_cases"])
    serialized = json.dumps(artifact).lower()
    assert all(
        forbidden not in serialized
        for forbidden in ("bearer ", "api_key", "/users/", "sqlite3", "query_history")
    )
