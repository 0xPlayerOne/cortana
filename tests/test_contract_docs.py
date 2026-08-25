"""Contract-source guardrails for the M2 durable specifications."""

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
