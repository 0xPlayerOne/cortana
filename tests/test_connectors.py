from __future__ import annotations

import base64
import datetime as dt
import io
import json
import sqlite3
import subprocess
from pathlib import Path
from typing import Any

import httpx
import pytest

from cortana.connectors import __main__ as connector_cli
from cortana.connectors import apple_notes, buzz, chat
from cortana.connectors.__main__ import main
from cortana.connectors.google import (
    GoogleSession,
    _gmail_document,
    _plain_text,
    _timestamp,
    fetch_drive,
    fetch_gmail,
)
from cortana.connectors.model import Document, emit


def response(
    payload: Any, status: int = 200, request: httpx.Request | None = None
) -> httpx.Response:
    return httpx.Response(
        status, json=payload, request=request or httpx.Request("GET", "https://example.test")
    )


def test_document_jsonl_emit_uses_utc_and_skips_empty() -> None:
    output = io.StringIO()
    count = emit(
        [
            Document(
                source="test",
                source_id="1",
                title="Example",
                content="useful",
                updated_at=dt.datetime(2026, 1, 2, tzinfo=dt.timezone(dt.timedelta(hours=-4))),
                acl=("agent",),
            ),
            Document(source="test", source_id="2", title="Empty", content=" "),
        ],
        output,
    )

    assert count == 1
    payload = json.loads(output.getvalue())
    assert payload["updated_at"] == "2026-01-02T04:00:00+00:00"
    assert payload["acl"] == ["agent"]


def test_apple_notes_normalizes_jxa_rows(monkeypatch: pytest.MonkeyPatch) -> None:
    completed = subprocess.CompletedProcess(
        args=[],
        returncode=0,
        stdout=json.dumps(
            [
                {
                    "id": "x-coredata://note/1",
                    "name": "Plan",
                    "body": "Ship Cortana",
                    "modified": "2026-07-29T10:00:00.000Z",
                    "account": "iCloud",
                    "folder": "Notes",
                },
                {
                    "id": "x-coredata://note/2",
                    "name": "Blank",
                    "body": " ",
                    "modified": "2026-07-29T10:00:00.000Z",
                },
            ]
        ),
        stderr="",
    )
    monkeypatch.setattr(subprocess, "run", lambda *_args, **_kwargs: completed)

    documents = list(apple_notes.fetch(project="personal"))

    assert len(documents) == 1
    assert documents[0].source_id == "x-coredata://note/1"
    assert documents[0].metadata == {"account": "iCloud", "folder": "Notes"}


def test_buzz_reads_personas_and_logs_read_only(tmp_path: Path) -> None:
    agents = tmp_path / "agents"
    logs = agents / "logs"
    logs.mkdir(parents=True)
    database = agents / "retention.db"
    with sqlite3.connect(database) as connection:
        connection.execute(
            "CREATE TABLE persona_events("
            "kind INTEGER,pubkey TEXT,d_tag TEXT,content TEXT,created_at INTEGER,"
            "raw_event TEXT,pending_sync INTEGER)"
        )
        connection.execute(
            "INSERT INTO persona_events VALUES(?,?,?,?,?,?,?)",
            (30078, "pub", "profile", "Agent profile", 1_700_000_000, '{"id":"event"}', 0),
        )
    (logs / "agent.log").write_text("started agent", encoding="utf-8")

    documents = list(buzz.fetch(tmp_path))

    assert [document.source_id for document in documents] == [
        "persona:30078:pub:profile",
        "log:agent.log",
    ]
    assert documents[0].metadata["raw_event"]["id"] == "event"


class FakeSlackClient:
    def __init__(self, *_args: object, **_kwargs: object) -> None:
        self.calls: list[str] = []

    def __enter__(self) -> FakeSlackClient:
        return self

    def __exit__(self, *_args: object) -> None:
        pass

    def get(self, path: str, **_kwargs: object) -> httpx.Response:
        self.calls.append(path)
        if path == "/conversations.history":
            return response(
                {
                    "ok": True,
                    "messages": [{"ts": "10.0", "user": "U1", "text": "Launch?", "reply_count": 1}],
                    "response_metadata": {"next_cursor": ""},
                }
            )
        return response(
            {
                "ok": True,
                "messages": [
                    {"ts": "10.0", "user": "U1", "text": "Launch?"},
                    {"ts": "11.0", "user": "U2", "text": "Yes"},
                ],
            }
        )


class FakeDiscordClient:
    def __init__(self, *_args: object, **_kwargs: object) -> None:
        pass

    def __enter__(self) -> FakeDiscordClient:
        return self

    def __exit__(self, *_args: object) -> None:
        pass

    def get(self, _path: str, **_kwargs: object) -> httpx.Response:
        return response(
            [
                {
                    "id": "99",
                    "content": "Status",
                    "attachments": [{"url": "https://files.test/report.pdf"}],
                    "timestamp": "2026-07-29T12:00:00Z",
                    "author": {"id": "u1", "username": "Ada"},
                }
            ]
        )


def test_chat_connectors_reassemble_slack_and_normalize_discord(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SLACK_TEST_TOKEN", "secret")
    monkeypatch.setattr(chat.httpx, "Client", FakeSlackClient)
    slack_documents = list(chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN"))
    assert slack_documents[0].content == "U1: Launch?\nU2: Yes"
    assert slack_documents[0].metadata["participants"] == ["U1", "U2"]

    monkeypatch.setenv("DISCORD_TEST_TOKEN", "secret")
    monkeypatch.setattr(chat.httpx, "Client", FakeDiscordClient)
    discord_documents = list(chat.fetch_discord(["D1"], "work", "DISCORD_TEST_TOKEN"))
    assert "https://files.test/report.pdf" in discord_documents[0].content
    assert discord_documents[0].metadata["author_id"] == "u1"


def test_chat_connector_rejects_missing_token(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("MISSING_TOKEN", raising=False)
    with pytest.raises(RuntimeError, match="MISSING_TOKEN is required"):
        list(chat.fetch_slack(["C1"], "work", "MISSING_TOKEN"))


def test_google_drive_exports_supported_content(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Roadmap",
                            "mimeType": "application/vnd.google-apps.document",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                            "webViewLink": "https://docs.google.com/document/d/doc1",
                            "owners": [{"displayName": "Ada"}],
                        },
                        {
                            "id": "bin1",
                            "name": "Image",
                            "mimeType": "image/png",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        },
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="Quarterly roadmap", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(fetch_drive(token, "work", client=client))

    assert len(documents) == 1
    assert documents[0].content == "Quarterly roadmap"
    assert documents[0].metadata["owners"] == ["Ada"]


def test_google_gmail_decodes_message_body(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")
    encoded = base64.urlsafe_b64encode(b"Deployment is green").decode()

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response({"messages": [{"id": "m1"}]}, request=request)
        return response(
            {
                "id": "m1",
                "threadId": "t1",
                "internalDate": "1700000000000",
                "labelIds": ["INBOX"],
                "payload": {
                    "headers": [
                        {"name": "From", "value": "ada@example.test"},
                        {"name": "To", "value": "team@example.test"},
                        {"name": "Subject", "value": "Release"},
                    ],
                    "mimeType": "text/plain",
                    "body": {"data": encoded},
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(fetch_gmail(token, "work", client=client))

    assert documents[0].title == "Release"
    assert "Deployment is green" in documents[0].content
    assert documents[0].metadata["thread_id"] == "t1"


def test_google_session_refreshes_and_secures_token_file(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text(
        json.dumps(
            {
                "refresh_token": "refresh",
                "client_id": "client",
                "client_secret": "secret",
                "token_uri": "https://oauth2.test/token",
            }
        ),
        encoding="utf-8",
    )

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.host == "oauth2.test":
            return response({"access_token": "new-access", "expires_in": 100}, request=request)
        return response({"ok": True}, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with GoogleSession(token, client) as session:
        assert session.request("GET", "https://api.test/data").json() == {"ok": True}

    saved = json.loads(token.read_text(encoding="utf-8"))
    assert saved["token"] == "new-access"
    assert token.stat().st_mode & 0o777 == 0o600


def test_google_session_retries_unauthorized_response(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text(
        json.dumps(
            {
                "token": "expired",
                "refresh_token": "refresh",
                "client_id": "client",
                "client_secret": "secret",
            }
        ),
        encoding="utf-8",
    )
    calls = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal calls
        if request.url.path == "/token":
            return response({"access_token": "fresh"}, request=request)
        calls += 1
        return response({"ok": True}, 401 if calls == 1 else 200, request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with GoogleSession(token, client) as session:
        assert session.request("GET", "https://api.test/data").json() == {"ok": True}
    assert calls == 2


def test_google_helpers_normalize_html_dates_and_snippets() -> None:
    assert _plain_text("<style>x{}</style><p>Hello &amp; world</p>", "text/html") == "Hello & world"
    assert _timestamp("Tue, 29 Jul 2026 12:00:00 +0000").year == 2026
    message = _gmail_document(
        {
            "id": "m2",
            "snippet": "Fallback text",
            "payload": {
                "headers": [
                    {"name": "Subject", "value": "Fallback"},
                    {"name": "Date", "value": "Tue, 29 Jul 2026 12:00:00 +0000"},
                ]
            },
        },
        "personal",
    )
    assert "Fallback text" in message.content
    assert message.uri.endswith("/m2")


def test_google_session_reports_unrefreshable_credentials(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text("{}", encoding="utf-8")
    with (
        GoogleSession(token, httpx.Client()) as session,
        pytest.raises(RuntimeError, match="missing refresh_token"),
    ):
        session.request("GET", "https://api.test/data")


def test_connector_cli_emits_buzz_jsonl(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    logs = tmp_path / "agents" / "logs"
    logs.mkdir(parents=True)
    (logs / "agent.log").write_text("event", encoding="utf-8")

    assert main(["--project", "agents", "buzz", "--root", str(tmp_path)]) == 0

    captured = capsys.readouterr()
    assert json.loads(captured.out)["project"] == "agents"
    assert "emitted=1" in captured.err


def test_connector_cli_dispatches_chat_sources(monkeypatch: pytest.MonkeyPatch) -> None:
    expected = [Document(source="test", source_id="1", title="One", content="Body")]
    monkeypatch.setattr(connector_cli, "fetch_slack", lambda *_args: expected)
    monkeypatch.setattr(connector_cli, "fetch_discord", lambda *_args: expected)

    slack_args = connector_cli.parser().parse_args(
        ["--project", "work", "slack", "--channel", "C1"]
    )
    discord_args = connector_cli.parser().parse_args(
        ["--project", "work", "discord", "--channel", "D1"]
    )
    assert list(connector_cli._documents(slack_args)) == expected
    assert list(connector_cli._documents(discord_args)) == expected


def test_connector_cli_requires_existing_google_token(tmp_path: Path) -> None:
    args = connector_cli.parser().parse_args(
        ["google-drive", "--token", str(tmp_path / "missing.json")]
    )
    with pytest.raises(RuntimeError, match="does not exist"):
        connector_cli._documents(args)
