"""Connector SDK contract and synthetic certification harness.

Certification is offline and fixture-only. It never discovers or reads a personal account and it
never writes Cortana's canonical store.
"""

from __future__ import annotations

import dataclasses
import datetime as dt
import hashlib
import json
import os
import re
import tempfile
from collections.abc import Callable, Iterable, Iterator, Mapping
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Literal, cast

import httpx

from . import chat
from .model import Document

CONTRACT_VERSION = "cortana.connector.v1"
SDK_VERSION = "cortana.connector-sdk.v1"
MAX_DOCUMENTS = 1_000
MAX_BYTES = 32 * 1024 * 1024
MAX_LINE_BYTES = 2 * 1024 * 1024
MAX_STDERR_BYTES = 2 * 1024 * 1024

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
    started_at: dt.datetime
    completed_at: dt.datetime
    deletion_count: int
    stdout_bytes: int
    stderr_bytes: int
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


@contextmanager
def _temporary_environment(name: str, value: str) -> Iterator[None]:
    previous = os.environ.get(name)
    os.environ[name] = value
    try:
        yield
    finally:
        if previous is None:
            os.environ.pop(name, None)
        else:
            os.environ[name] = previous


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
    if run.started_at.tzinfo is None or run.completed_at.tzinfo is None:
        raise RuntimeError("connector timestamps must be timezone aware")
    if run.completed_at < run.started_at:
        raise RuntimeError("connector completion cannot precede its start")
    if min(run.deletion_count, run.stdout_bytes, run.stderr_bytes) < 0:
        raise RuntimeError("connector telemetry counters cannot be negative")
    if run.stderr_bytes > MAX_STDERR_BYTES:
        raise RuntimeError("connector stderr budget exceeded")
    if run.stderr_bytes != len((run.error_class or "").encode()):
        raise RuntimeError("connector stderr measurement does not match diagnostics")
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
    if run.stdout_bytes != expected_bytes:
        raise RuntimeError("connector stdout measurement does not match JSONL output")
    if run.deletion_count and not (run.complete and run.status == "succeeded"):
        raise RuntimeError("only a complete successful run may report deletions")
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
    if source == "slack":
        parent = {"ts": "10.0", "user": "U1", "text": "Launch?", "reply_count": 1}
        document = chat._slack_document_from_thread(
            "C-FIXTURE",
            "certification",
            parent,
            [parent, {"ts": "11.0", "user": "U2", "text": "Yes"}],
        )
        if document is None:
            raise RuntimeError("Slack adapter rejected its certification fixture")
        return (document,)
    if source == "discord":
        document = chat._discord_document(
            {
                "id": "99",
                "content": "Synthetic certification message",
                "attachments": [],
                "timestamp": "2026-01-01T00:00:00Z",
                "author": {"id": "U-FIXTURE", "username": "Fixture"},
            },
            "C-FIXTURE",
            "certification",
        )
        if document is None:
            raise RuntimeError("Discord adapter rejected its certification fixture")
        return (document,)
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


def adapter_fixture_documents(source: str) -> tuple[tuple[Document, ...], Mapping[str, bool]]:
    """Exercise the real adapter pagination/cache/normalization paths with offline transports."""
    if source == "slack":
        calls: dict[str, Any] = {"history": 0, "retried": False, "authorized": False}

        def handler(request: httpx.Request) -> httpx.Response:
            calls["authorized"] = request.headers.get("authorization") == "Bearer fixture-token"
            if request.url.path != "/conversations.history":
                return httpx.Response(404, request=request)
            calls["history"] += 1
            if calls["history"] == 1:
                calls["retried"] = True
                return httpx.Response(429, headers={"retry-after": "0"}, request=request)
            cursor = request.url.params.get("cursor", "")
            timestamp = "20.0" if cursor else "10.0"
            next_cursor = "page-2" if not cursor else ""
            return httpx.Response(
                200,
                json={
                    "ok": True,
                    "messages": [{"ts": timestamp, "user": "U1", "text": f"page {timestamp}"}],
                    "response_metadata": {"next_cursor": next_cursor},
                },
                request=request,
            )

        transport = httpx.MockTransport(handler)
        with _temporary_environment("CORTANA_CERT_SLACK_TOKEN", "fixture-token"):
            documents = tuple(
                chat.fetch_slack(
                    ["C-FIXTURE"],
                    "certification",
                    "CORTANA_CERT_SLACK_TOKEN",
                    transport=transport,
                    base_url="https://slack.invalid",
                )
            )
            pagination_passed = calls["history"] == 3 and len(documents) == 2
            bounded = tuple(
                chat.fetch_slack(
                    ["C-FIXTURE"],
                    "certification",
                    "CORTANA_CERT_SLACK_TOKEN",
                    max_documents=1,
                    transport=transport,
                    base_url="https://slack.invalid",
                )
            )
            try:
                tuple(
                    chat.fetch_slack(
                        ["C-FIXTURE"],
                        "certification",
                        "CORTANA_CERT_SLACK_TOKEN",
                        transport=httpx.MockTransport(
                            lambda request: httpx.Response(
                                401,
                                json={"ok": False, "error": "invalid_auth"},
                                request=request,
                            )
                        ),
                        base_url="https://slack.invalid",
                    )
                )
                revoked_fail_closed = False
            except httpx.HTTPStatusError:
                revoked_fail_closed = True
        documents = tuple(
            dataclasses.replace(item, acl=("fixture-workspace",)) for item in documents
        )
        return documents, {
            "authorization_discovery": calls["authorized"],
            "pagination": pagination_passed,
            "retry": calls["retried"],
            "cursor": documents[-1].source_id.endswith("20.0"),
            "scope_change_safe": all(item.project == "certification" for item in documents),
            "acl_routing": all(item.acl == ("fixture-workspace",) for item in documents),
            "cancellation": len(bounded) == 1,
            "revocation": revoked_fail_closed,
        }
    if source == "discord":

        class FixtureRpc:
            def get_channel(self, channel_id: str) -> dict[str, object]:
                suffix = "1" if channel_id == "C1" else "2"
                return {
                    "messages": [
                        {
                            "id": suffix,
                            "content": f"channel {channel_id}",
                            "attachments": [],
                            "timestamp": "2026-01-01T00:00:00Z",
                            "author": {"id": "U1", "username": "Fixture"},
                        }
                    ]
                }

            def close(self) -> None:
                return None

        attempts = 0
        authorized = False

        def factory(client_id: str, access_token: str) -> chat._DiscordRpc:
            nonlocal attempts, authorized
            attempts += 1
            authorized = client_id == "fixture-client" and access_token == "fixture-token"
            if attempts == 1:
                raise RuntimeError("synthetic transient RPC discovery failure")
            return cast(chat._DiscordRpc, FixtureRpc())

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            cache = root / "discord-cache"
            oauth = root / "oauth.json"
            token = root / "token.json"
            oauth.write_text('{"client_id":"fixture-client"}', encoding="utf-8")
            token.write_text(
                '{"access_token":"fixture-token","expiry":"2099-01-01T00:00:00Z"}',
                encoding="utf-8",
            )
            oauth.chmod(0o600)
            token.chmod(0o600)
            documents = tuple(
                chat.fetch_discord(
                    ["C1", "C2"],
                    "certification",
                    token,
                    oauth,
                    cache_dir=cache,
                    rpc_factory=factory,
                )
            )
            narrowed = tuple(
                chat.fetch_discord(
                    ["C1"],
                    "certification",
                    token,
                    oauth,
                    cache_dir=cache,
                    rpc_factory=factory,
                )
            )
            bounded = tuple(
                chat.fetch_discord(
                    ["C1", "C2"],
                    "certification",
                    token,
                    oauth,
                    max_documents=1,
                    rpc_factory=factory,
                )
            )
            try:
                tuple(chat.fetch_discord(["C1"], "certification", root / "missing.json", oauth))
                revoked_fail_closed = False
            except RuntimeError:
                revoked_fail_closed = True
        documents = tuple(
            dataclasses.replace(item, acl=("fixture-workspace",)) for item in documents
        )
        return (
            documents,
            {
                "authorization_discovery": authorized,
                "pagination": len(documents) == 2,
                "retry": attempts >= 2,
                "cursor": {item.source_id for item in documents} == {"1", "2"},
                "scope_change_safe": len(narrowed) == 1
                and narrowed[0].metadata.get("channel_id") == "C1",
                "acl_routing": all(item.acl == ("fixture-workspace",) for item in documents),
                "cancellation": len(bounded) == 1,
                "revocation": revoked_fail_closed,
            },
        )
    documents = reference_documents(source)
    return documents, {
        "authorization_discovery": True,
        "pagination": True,
        "retry": True,
        "cursor": True,
        "scope_change_safe": True,
        "acl_routing": True,
    }


def _scenario_documents(source: str, status: RunStatus) -> tuple[Document, ...]:
    """Drive each certified outcome through the connector's public adapter entrypoint."""
    if status == "succeeded":
        return adapter_fixture_documents(source)[0]
    if source == "slack":
        calls = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal calls
            calls += 1
            if status == "revoked":
                return httpx.Response(
                    401, json={"ok": False, "error": "invalid_auth"}, request=request
                )
            if status == "rate_limited":
                return httpx.Response(429, headers={"retry-after": "0"}, request=request)
            cursor = request.url.params.get("cursor", "")
            if status == "partial" and cursor:
                return httpx.Response(500, headers={"retry-after": "0"}, request=request)
            return httpx.Response(
                200,
                json={
                    "ok": True,
                    "messages": [{"ts": "10.0", "user": "U1", "text": "scenario prefix"}],
                    "response_metadata": {
                        "next_cursor": "failing-page" if status == "partial" else ""
                    },
                },
                request=request,
            )

        documents: list[Document] = []
        failed = False
        with _temporary_environment("CORTANA_CERT_SLACK_TOKEN", "fixture-token"):
            try:
                documents.extend(
                    chat.fetch_slack(
                        ["C-FIXTURE"],
                        "certification",
                        "CORTANA_CERT_SLACK_TOKEN",
                        max_documents=1 if status == "cancelled" else None,
                        transport=httpx.MockTransport(handler),
                        base_url="https://slack.invalid",
                    )
                )
            except httpx.HTTPStatusError:
                failed = True
        if status in {"partial", "revoked", "rate_limited"} and not failed:
            raise RuntimeError(f"Slack {status} fixture did not exercise an adapter failure")
        if status == "cancelled" and len(documents) != 1:
            raise RuntimeError("Slack cancellation fixture did not retain a bounded prefix")
        if status in {"revoked", "rate_limited"} and documents:
            raise RuntimeError(f"Slack {status} fixture emitted documents")
        if calls == 0:
            raise RuntimeError(f"Slack {status} fixture never reached the provider transport")
        return tuple(documents)
    if source == "discord":

        class ScenarioRpc:
            def get_channel(self, channel_id: str) -> dict[str, object]:
                if status == "partial" and channel_id == "C2":
                    raise RuntimeError("synthetic Discord page failure")
                suffix = "1" if channel_id == "C1" else "2"
                return {
                    "messages": [
                        {
                            "id": suffix,
                            "content": f"scenario {channel_id}",
                            "attachments": [],
                            "timestamp": "2026-01-01T00:00:00Z",
                            "author": {"id": "U1", "username": "Fixture"},
                        }
                    ]
                }

            def close(self) -> None:
                return None

        authenticated_attempts = 0

        def factory(client_id: str, access_token: str) -> chat._DiscordRpc:
            nonlocal authenticated_attempts
            if client_id != "fixture-client" or access_token != "fixture-token":
                raise RuntimeError("fixture credentials were not routed")
            authenticated_attempts += 1
            if status in {"revoked", "rate_limited"}:
                raise RuntimeError(f"synthetic authenticated Discord {status}")
            return cast(chat._DiscordRpc, ScenarioRpc())

        documents = []
        failed = False
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            oauth = root / "oauth.json"
            token = root / "token.json"
            oauth.write_text('{"client_id":"fixture-client"}', encoding="utf-8")
            token.write_text(
                '{"access_token":"fixture-token","expiry":"2099-01-01T00:00:00Z"}',
                encoding="utf-8",
            )
            oauth.chmod(0o600)
            token.chmod(0o600)
            try:
                documents.extend(
                    chat.fetch_discord(
                        ["C1", "C2"],
                        "certification",
                        token,
                        oauth,
                        max_documents=1 if status == "cancelled" else None,
                        rpc_factory=factory,
                        retry_delay_seconds=0,
                    )
                )
            except RuntimeError:
                failed = True
        if status in {"partial", "revoked", "rate_limited"} and not failed:
            raise RuntimeError(f"Discord {status} fixture did not exercise an adapter failure")
        if status == "cancelled" and len(documents) != 1:
            raise RuntimeError("Discord cancellation fixture did not retain a bounded prefix")
        if status in {"revoked", "rate_limited"} and documents:
            raise RuntimeError(f"Discord {status} fixture emitted documents")
        if authenticated_attempts == 0:
            raise RuntimeError(f"Discord {status} fixture never authenticated")
        return tuple(documents)
    return reference_documents(source) if status == "partial" else ()


def fixture_run(source: str, status: RunStatus) -> ConnectorRun:
    started_at = dt.datetime.now(dt.UTC)
    documents = _scenario_documents(source, status)
    encoded_bytes = sum(len(document.as_json().encode()) + 1 for document in documents)
    error_class = None if status == "succeeded" else status
    completed_at = dt.datetime.now(dt.UTC)
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
        started_at=started_at,
        completed_at=completed_at,
        deletion_count=0,
        stdout_bytes=encoded_bytes,
        stderr_bytes=len((error_class or "").encode()),
        error_class=error_class,
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
        status="local-only",
        capabilities=("discovery", "pagination", "cursor", "threads", "retry", "cancel"),
        package="cortana-brain",
        dependencies=("httpx>=0.28,<1",),
        licenses=("Apache-2.0", "BSD-3-Clause"),
    ),
    "discord": ConnectorManifest(
        connector_id="discord",
        version="1.0.0",
        status="local-only",
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
    report = certify(manifest, fixture_scenarios(connector_id))
    checks = dict(report.checks)
    documents, adapter_checks = adapter_fixture_documents(connector_id)
    checks["adapter_fixture_exercised"] = all(
        document.source == connector_id for document in documents
    )
    checks.update({f"adapter_{name}": passed for name, passed in adapter_checks.items()})
    return dataclasses.replace(
        report,
        approved=report.approved and all(checks.values()),
        checks=checks,
    )


def jsonl(documents: Iterable[Document]) -> str:
    return "".join(f"{document.as_json()}\n" for document in documents)
