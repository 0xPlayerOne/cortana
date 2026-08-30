"""Connector SDK contract and synthetic certification harness.

Certification is offline and fixture-only. It never discovers or reads a personal account and it
never writes Cortana's canonical store.
"""

from __future__ import annotations

import dataclasses
import datetime as dt
import hashlib
import json
import re
from collections.abc import Callable, Iterable, Mapping
from typing import Literal

from .model import Document

CONTRACT_VERSION = "cortana.connector.v1"
SDK_VERSION = "cortana.connector-sdk.v1"
MAX_DOCUMENTS = 1_000
MAX_BYTES = 32 * 1024 * 1024
MAX_LINE_BYTES = 2 * 1024 * 1024

SupportStatus = Literal["supported", "experimental", "local-only", "rejected"]
RunStatus = Literal["succeeded", "partial", "cancelled", "rate_limited", "revoked", "failed"]


@dataclasses.dataclass(frozen=True)
class ConnectorManifest:
    connector_id: str
    version: str
    status: SupportStatus
    capabilities: tuple[str, ...]
    package: str
    dependencies: tuple[str, ...]
    licenses: tuple[str, ...]
    enabled_by_default: bool = False
    contract_version: str = CONTRACT_VERSION
    sdk_version: str = SDK_VERSION


@dataclasses.dataclass(frozen=True)
class ConnectorRun:
    documents: tuple[Document, ...]
    status: RunStatus
    complete: bool
    cursor: str | None
    configuration_fingerprint: str
    progress_documents: int
    progress_bytes: int
    error_class: str | None = None
    cancelled: bool = False


@dataclasses.dataclass(frozen=True)
class ValidatedRun:
    run: ConnectorRun
    reconcile_allowed: bool
    deterministic_ids: tuple[str, ...]


@dataclasses.dataclass(frozen=True)
class CertificationReport:
    connector_id: str
    manifest_version: str
    contract_version: str
    support_status: SupportStatus
    approved: bool
    fixture_only: bool
    checks: Mapping[str, bool]

    def as_json(self) -> str:
        return json.dumps(dataclasses.asdict(self), sort_keys=True, separators=(",", ":"))


def stable_configuration_fingerprint(config: Mapping[str, object]) -> str:
    encoded = json.dumps(config, sort_keys=True, separators=(",", ":")).encode()
    return f"sha256:{hashlib.sha256(encoded).hexdigest()}"


def stable_document_id(document: Document) -> str:
    encoded = f"{document.source}\0{document.source_id}".encode()
    return f"sha256:{hashlib.sha256(encoded).hexdigest()}"


def validate_run(manifest: ConnectorManifest, run: ConnectorRun) -> ValidatedRun:
    if manifest.contract_version != CONTRACT_VERSION or manifest.sdk_version != SDK_VERSION:
        raise RuntimeError("connector contract or SDK version is incompatible")
    if manifest.enabled_by_default:
        raise RuntimeError("external connectors must be disabled by default")
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{1,63}", manifest.connector_id):
        raise RuntimeError("connector_id is invalid")
    if not manifest.version or not manifest.capabilities or not manifest.package:
        raise RuntimeError("connector manifest is incomplete")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", run.configuration_fingerprint):
        raise RuntimeError("configuration fingerprint is invalid")
    if run.cursor is not None and (
        len(run.cursor) > 4_096 or any(ord(character) < 32 for character in run.cursor)
    ):
        raise RuntimeError("connector cursor is invalid")
    if run.progress_documents < 0 or run.progress_bytes < 0:
        raise RuntimeError("connector progress counters cannot be negative")
    if len(run.documents) > MAX_DOCUMENTS:
        raise RuntimeError("connector document budget exceeded")
    expected_bytes = 0
    identities: list[str] = []
    seen: set[tuple[str, str]] = set()
    for document in run.documents:
        if not document.source or not document.source_id or not document.content.strip():
            raise RuntimeError("connector emitted an incomplete Document")
        if (document.source, document.source_id) in seen:
            raise RuntimeError("connector emitted a duplicate source identity")
        seen.add((document.source, document.source_id))
        line = document.as_json().encode()
        if len(line) > MAX_LINE_BYTES:
            raise RuntimeError("connector JSONL row exceeds the line budget")
        if any(
            key in line.lower()
            for key in (b"authorization: bearer", b"refresh_token", b"client_secret")
        ):
            raise RuntimeError("connector output contains credential-shaped data")
        if any(marker in line for marker in (b'"file:///', b'"/Users/', b'"/home/')):
            raise RuntimeError("connector output contains a private absolute path")
        if any(not isinstance(scope, str) or not scope.strip() for scope in document.acl):
            raise RuntimeError("connector emitted an invalid ACL")
        expected_bytes += len(line) + 1
        identities.append(stable_document_id(document))
    if expected_bytes > MAX_BYTES:
        raise RuntimeError("connector byte budget exceeded")
    if run.progress_documents != len(run.documents) or run.progress_bytes != expected_bytes:
        raise RuntimeError("connector progress counters do not match output")
    if run.complete != (run.status == "succeeded"):
        raise RuntimeError("only a succeeded run may declare a complete snapshot")
    if run.cancelled != (run.status == "cancelled"):
        raise RuntimeError("cancellation status is inconsistent")
    reconcile_allowed = run.complete and run.status == "succeeded"
    return ValidatedRun(run, reconcile_allowed, tuple(identities))


def certify(
    manifest: ConnectorManifest,
    scenarios: Mapping[str, Callable[[], ConnectorRun]],
) -> CertificationReport:
    required = {"complete", "partial", "cancelled", "revoked", "rate_limited"}
    checks: dict[str, bool] = {
        "manifest_compatible": manifest.contract_version == CONTRACT_VERSION
        and manifest.sdk_version == SDK_VERSION,
        "disabled_by_default": not manifest.enabled_by_default,
        "required_scenarios": required.issubset(scenarios),
        "fixture_only": True,
    }
    validated: dict[str, ValidatedRun] = {}
    for name in sorted(required.intersection(scenarios)):
        try:
            validated[name] = validate_run(manifest, scenarios[name]())
            checks[f"scenario_{name}"] = True
        except (RuntimeError, ValueError, TypeError):
            checks[f"scenario_{name}"] = False
    complete = validated.get("complete")
    try:
        repeated = validate_run(manifest, scenarios["complete"]())
    except (RuntimeError, ValueError, TypeError, KeyError):
        repeated = None
    checks["identity_deterministic"] = (
        complete is not None
        and repeated is not None
        and complete.deterministic_ids == repeated.deterministic_ids
        and tuple(document.as_json() for document in complete.run.documents)
        == tuple(document.as_json() for document in repeated.run.documents)
    )
    checks["complete_reconciles"] = complete is not None and complete.reconcile_allowed
    checks["failures_do_not_reconcile"] = all(
        not validated[name].reconcile_allowed
        for name in required - {"complete"}
        if name in validated
    ) and all(name in validated for name in required - {"complete"})
    approved = all(checks.values()) and manifest.status != "rejected"
    return CertificationReport(
        connector_id=manifest.connector_id,
        manifest_version=manifest.version,
        contract_version=manifest.contract_version,
        support_status=manifest.status,
        approved=approved,
        fixture_only=True,
        checks=checks,
    )


def reference_documents(source: str = "external-reference") -> tuple[Document, ...]:
    return (
        Document(
            source=source,
            source_id="fixture:item-1",
            title="Synthetic connector fixture",
            content="Synthetic evidence used only by the offline certification harness.",
            updated_at=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
            project="certification",
            acl=("fixture",),
            metadata={"fixture": True, "cursor": "page-1"},
        ),
    )


def fixture_run(source: str, status: RunStatus) -> ConnectorRun:
    documents = reference_documents(source) if status in {"succeeded", "partial"} else ()
    encoded_bytes = sum(len(document.as_json().encode()) + 1 for document in documents)
    return ConnectorRun(
        documents=documents,
        status=status,
        complete=status == "succeeded",
        cursor="fixture-cursor" if documents else None,
        configuration_fingerprint=stable_configuration_fingerprint(
            {"source": source, "fixture": True}
        ),
        progress_documents=len(documents),
        progress_bytes=encoded_bytes,
        error_class=None if status == "succeeded" else status,
        cancelled=status == "cancelled",
    )


def fixture_scenarios(source: str) -> Mapping[str, Callable[[], ConnectorRun]]:
    statuses: Mapping[str, RunStatus] = {
        "complete": "succeeded",
        "partial": "partial",
        "cancelled": "cancelled",
        "revoked": "revoked",
        "rate_limited": "rate_limited",
    }

    def scenario(status: RunStatus) -> Callable[[], ConnectorRun]:
        def run() -> ConnectorRun:
            return fixture_run(source, status)

        return run

    return {name: scenario(status) for name, status in statuses.items()}


BUILTIN_MANIFESTS: Mapping[str, ConnectorManifest] = {
    "slack": ConnectorManifest(
        connector_id="slack",
        version="1.0.0",
        status="supported",
        capabilities=("discovery", "pagination", "cursor", "threads", "retry", "cancel"),
        package="cortana-brain",
        dependencies=("httpx>=0.28,<1",),
        licenses=("Apache-2.0", "BSD-3-Clause"),
    ),
    "discord": ConnectorManifest(
        connector_id="discord",
        version="1.0.0",
        status="supported",
        capabilities=("discovery", "pagination", "cursor", "threads", "retry", "cancel"),
        package="cortana-brain",
        dependencies=("httpx>=0.28,<1",),
        licenses=("Apache-2.0", "BSD-3-Clause"),
    ),
    "external-reference": ConnectorManifest(
        connector_id="external-reference",
        version="1.0.0",
        status="experimental",
        capabilities=("snapshot", "cursor", "cancel", "progress"),
        package="connector-reference-adapter",
        dependencies=(),
        licenses=("Apache-2.0",),
    ),
}


def certify_builtin(connector_id: str) -> CertificationReport:
    try:
        manifest = BUILTIN_MANIFESTS[connector_id]
    except KeyError as error:
        raise RuntimeError(f"unknown certification manifest: {connector_id}") from error
    return certify(manifest, fixture_scenarios(connector_id))


def jsonl(documents: Iterable[Document]) -> str:
    return "".join(f"{document.as_json()}\n" for document in documents)
