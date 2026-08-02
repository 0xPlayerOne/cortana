from __future__ import annotations

import json

import httpx
import pytest

from cortana.memory import (
    MemoryArgumentError,
    MemoryDocument,
    MemoryError,
    MemorySyncWorker,
    Outbox,
    OutboxEntry,
    stable_document_id,
)
from cortana.memory.hindsight import HindsightConfig, HindsightHttpProvider
from cortana.memory.models import workspace_acl_tags
from cortana.memory.provider import ProviderError


def test_stable_id_and_strict_tag_mapping() -> None:
    assert stable_document_id("Personal", "gmail", "thread/1") == stable_document_id(
        "personal", "gmail", "thread/1"
    )
    assert workspace_acl_tags("Personal", ("owner", "team-read")) == [
        "acl:owner",
        "acl:team-read",
        "workspace:personal",
    ]


def test_memory_document_validation() -> None:
    with pytest.raises(MemoryError, match="malformed"):
        MemoryDocument(
            project="Personal",
            source="GMail",
            source_id="1",
            title="t",
            content="x",
            acl=("bad/tag",),
        )
    with pytest.raises(MemoryError, match="malformed"):
        MemoryDocument(project="personal", source="gmail", source_id="a b", title="t", content="x")


def test_hindsight_retain_shape_and_bank_scoped_delete() -> None:
    calls: list[tuple[str, str, dict[str, object]]] = []

    def handler(request: httpx.Request) -> httpx.Response:
        if request.method == "POST":
            payload = json.loads(request.content.decode("utf-8"))
            calls.append(("POST", request.url.path, payload))
            return httpx.Response(200, request=request)
        if request.method == "DELETE":
            calls.append(("DELETE", request.url.path, {}))
            return httpx.Response(200, request=request)
        return httpx.Response(404, request=request)

    transport = httpx.MockTransport(handler)
    provider = HindsightHttpProvider(
        HindsightConfig(
            base_url="https://example.test/api", bank="knowledge", token="secret-token"
        ),
        client=httpx.Client(transport=transport),
    )

    document = MemoryDocument(
        project="Work",
        source="gmail",
        source_id="thread/1",
        title="A",
        content="Hello",
        context="Summary snippet",
        metadata={"kind": "episode"},
        acl=("owner",),
    )

    provider.retain(document)
    provider.delete(document.document_id)
    provider.close()

    assert len(calls) == 2
    assert calls[0][0] == "POST"
    assert calls[0][1] == "/api/v1/default/banks/knowledge/memories/retain"
    payload = calls[0][2]
    assert payload["document_id"] == document.document_id
    assert payload["content"] == "Hello"
    assert payload["context"] == "Summary snippet"
    assert payload["metadata"] == {"kind": "episode"}
    assert payload["tags"] == ["acl:owner", "workspace:work"]

    assert calls[1][0] == "DELETE"
    assert calls[1][1] == f"/api/v1/default/banks/knowledge/documents/{document.document_id}"
    assert "secret-token" not in str(provider.diagnostics())


def test_hindsight_invalid_config_and_request_errors_are_retriable_and_opaque() -> None:
    with pytest.raises(MemoryArgumentError):
        HindsightHttpProvider(HindsightConfig(base_url="", bank="b", token="t"))
    with pytest.raises(MemoryArgumentError):
        HindsightHttpProvider(HindsightConfig(base_url="https://example.test", bank="", token="t"))
    with pytest.raises(MemoryArgumentError):
        HindsightHttpProvider(HindsightConfig(base_url="https://example.test", bank="b", token=""))
    with pytest.raises(MemoryArgumentError):
        HindsightHttpProvider(
            HindsightConfig(base_url="https://user:pass@example.test", bank="b", token="t")
        )
    with pytest.raises(MemoryArgumentError, match="HTTPS"):
        HindsightHttpProvider(
            HindsightConfig(base_url="http://remote.example.test", bank="b", token="t")
        )
    with pytest.raises(MemoryArgumentError):
        HindsightHttpProvider(
            HindsightConfig(base_url="https://example.test/?token=secret", bank="b", token="t")
        )
    with pytest.raises(MemoryArgumentError):
        HindsightHttpProvider(
            HindsightConfig(base_url="https://example.test", bank="bad/bank", token="t")
        )

    def failing(_: httpx.Request) -> httpx.Response:
        raise httpx.NetworkError("down")

    provider = HindsightHttpProvider(
        HindsightConfig(base_url="https://example.test", bank="knowledge", token="secret-token"),
        client=httpx.Client(transport=httpx.MockTransport(failing)),
    )
    doc = MemoryDocument(
        project="work",
        source="gmail",
        source_id="thread/1",
        title="A",
        content="Hello",
    )
    with pytest.raises(ProviderError) as exc:
        provider.retain(doc)
    assert "request failed" in str(exc.value)
    assert "secret-token" not in str(exc.value)


def test_outbox_upsert_retain_dedup_and_delete_entries(tmp_path) -> None:
    path = tmp_path / "outbox.sqlite3"
    doc = MemoryDocument(
        project="work",
        source="gmail",
        source_id="thread/1",
        title="Thread",
        content="alpha",
        context="inbox",
        metadata={"label": "inbox"},
        acl=("owner",),
    )

    with Outbox(path) as outbox:
        first = outbox.enqueue_retain(doc, max_attempts=3)
        second = outbox.enqueue_retain(doc, max_attempts=3)
        assert first == second

        pending = outbox.export_rows(states=("pending",), limit=10)
        assert len(pending) == 1
        assert isinstance(pending[0], OutboxEntry)

        delete_entry_id = outbox.enqueue_delete(
            project="work", source="gmail", source_id="thread/1", acl=("owner",)
        )
        delete_rows = outbox.export_rows(states=("pending",), limit=10)
        assert len(delete_rows) == 2
        assert any(row.operation == "delete" for row in delete_rows)
        assert any(row.id == delete_entry_id for row in delete_rows)

        entry = outbox.get_entry(document_id=doc.document_id, operation="retain")
        assert entry is not None
        assert entry.content == "alpha"

        with pytest.raises(MemoryError):
            outbox.enqueue_retain(
                MemoryDocument(project="", source="gmail", source_id="id", title="x", content="c")
            )


def test_worker_retries_and_dead_letters_without_private_db_access(tmp_path) -> None:
    path = tmp_path / "retry.sqlite3"

    class FailingProvider:
        def __init__(self) -> None:
            self.calls = 0

        @property
        def configured(self) -> bool:
            return True

        def retain(self, _document: MemoryDocument) -> None:
            self.calls += 1
            raise ProviderError("temporary", retriable=True)

        def delete(self, _document_id: str) -> None:
            raise AssertionError("delete should not run")

    provider = FailingProvider()
    document = MemoryDocument(
        project="work",
        source="gmail",
        source_id="thread/3",
        title="Thread",
        content="alpha",
        acl=("owner",),
    )

    with Outbox(path) as outbox:
        outbox.enqueue_retain(document, max_attempts=2)
        worker = MemorySyncWorker(outbox=outbox, provider=provider)
        worker.run(limit=1)

        assert outbox.stats()["pending"] == 1
        assert outbox.stats()["dead_letter"] == 0
        assert provider.calls == 1

        outbox.set_available(
            document_id=document.document_id, operation="retain", available_after=0.0
        )
        worker.run(limit=1)

        assert provider.calls == 2
        assert outbox.stats()["dead_letter"] == 1
        dead_rows = outbox.export_rows(states=("dead_letter",), limit=10)
        assert dead_rows[0].document_id == document.document_id


def test_worker_processes_delete_without_content_payload(tmp_path) -> None:
    path = tmp_path / "delete.sqlite3"

    class RecordingProvider:
        def __init__(self) -> None:
            self.deleted: list[str] = []

        @property
        def configured(self) -> bool:
            return True

        def retain(self, _document: MemoryDocument) -> None:
            raise AssertionError("retain should not run")

        def delete(self, document_id: str) -> None:
            self.deleted.append(document_id)

    provider = RecordingProvider()

    with Outbox(path) as outbox:
        outbox.enqueue_delete(project="work", source="gmail", source_id="thread/99", acl=("owner",))
        worker = MemorySyncWorker(outbox=outbox, provider=provider)
        worker.run(limit=1)

        assert provider.deleted == [stable_document_id("work", "gmail", "thread/99")]
        assert outbox.stats()["succeeded"] == 1

        with pytest.raises(MemoryArgumentError):
            outbox.enqueue_delete(project="work", source="gmail", source_id="bad id")


def test_outbox_telemetry_is_bounded_and_does_not_include_document_content(tmp_path) -> None:
    path = tmp_path / "telemetry.sqlite3"
    document = MemoryDocument(
        project="work",
        source="gmail",
        source_id="thread/telemetry",
        title="Private title",
        content="Private content",
    )

    class RecordingProvider:
        @property
        def configured(self) -> bool:
            return True

        def retain(self, _document: MemoryDocument) -> None:
            return None

        def delete(self, _document_id: str) -> None:
            return None

    with Outbox(path) as outbox:
        outbox.enqueue_retain(document)
        pending = outbox.telemetry()
        assert pending["queue_depth"] == 1
        assert pending["last_success_at"] is None
        assert pending["last_error"] is None
        assert "content" not in pending
        assert "Private content" not in str(pending)

        MemorySyncWorker(outbox=outbox, provider=RecordingProvider()).run(limit=1)
        succeeded = outbox.telemetry()
        assert succeeded["queue_depth"] == 0
        assert succeeded["succeeded"] == 1
        assert isinstance(succeeded["last_success_at"], float)

        outbox.mark_failed(
            outbox.enqueue_delete(project="work", source="gmail", source_id="thread/error"),
            error="line one\n" + "x" * 2_000,
            retriable=False,
        )
        failed = outbox.telemetry()
        assert failed["dead_letter"] == 1
        assert failed["last_error"] is not None
        assert len(str(failed["last_error"])) <= 512
        assert "\n" not in str(failed["last_error"])


def test_outbox_validation_for_limits_and_leases(tmp_path) -> None:
    path = tmp_path / "bounds.sqlite3"
    with Outbox(path) as outbox:
        with pytest.raises(MemoryArgumentError):
            outbox.claim_due(limit=0)
        with pytest.raises(MemoryArgumentError):
            outbox.claim_due(lease_seconds=0.0)
        with pytest.raises(MemoryArgumentError):
            outbox.export_rows(limit=0)
        with pytest.raises(MemoryArgumentError):
            outbox.enqueue_retain(
                MemoryDocument(
                    project="work", source="gmail", source_id="id", title="t", content="c"
                ),
                max_attempts=0,
            )
