from __future__ import annotations

import json

import pytest

from cortana.memory import MemoryDocument, Outbox, cli
from cortana.memory.honcho import HonchoConfig
from cortana.memory.provider import ProviderError


def test_memory_sync_parser_requires_an_explicit_provider_and_outbox() -> None:
    arguments = cli.parser().parse_args(
        ["--provider", "honcho", "--outbox", "/tmp/cortana-memory.sqlite3"]
    )
    assert arguments.provider == "honcho"
    assert arguments.limit == 64
    with pytest.raises(SystemExit):
        cli.parser().parse_args(["--provider", "honcho"])


def test_memory_sync_parser_bounds_batch_and_lease_inputs() -> None:
    parser = cli.parser()
    assert (
        parser.parse_args(
            [
                "--provider",
                "hindsight",
                "--outbox",
                "/tmp/cortana-memory.sqlite3",
                "--limit",
                "1024",
                "--lease-seconds",
                "3600",
            ]
        ).limit
        == 1024
    )
    for argument in (
        ["--limit", "0"],
        ["--limit", "1025"],
        ["--lease-seconds", "nan"],
        ["--lease-seconds", "3601"],
    ):
        with pytest.raises(SystemExit):
            parser.parse_args(
                ["--provider", "hindsight", "--outbox", "/tmp/outbox.sqlite3", *argument]
            )


def test_memory_sync_drains_honcho_outbox_without_printing_credentials(
    tmp_path, monkeypatch, capsys
) -> None:
    class RecordingProvider:
        configured = True

        def __init__(self) -> None:
            self.documents: list[MemoryDocument] = []
            self.closed = False

        def retain(self, document: MemoryDocument) -> None:
            self.documents.append(document)

        def delete(self, _document_id: str) -> None:
            raise ProviderError("unexpected delete")

        def close(self) -> None:
            self.closed = True

    provider = RecordingProvider()
    configs: list[HonchoConfig] = []

    def build(config: HonchoConfig) -> RecordingProvider:
        configs.append(config)
        return provider

    monkeypatch.setattr(cli, "HonchoHttpProvider", build)
    monkeypatch.setenv("HONCHO_TOKEN", "secret-token")
    outbox_path = tmp_path / "memory.sqlite3"
    document = MemoryDocument(
        project="work", source="notes", source_id="note-1", title="Title", content="Content"
    )
    with Outbox(outbox_path) as outbox:
        outbox.enqueue_retain(document)

    assert (
        cli.main(
            [
                "--provider",
                "honcho",
                "--allow-append-only",
                "--outbox",
                str(outbox_path),
                "--token-env",
                "HONCHO_TOKEN",
                "--workspace-id",
                "personal",
                "--peer-id",
                "agent",
                "--session-prefix",
                "brain",
            ]
        )
        == 0
    )
    output = capsys.readouterr().out
    payload = json.loads(output)
    assert payload["provider"] == "honcho"
    assert payload["processed"] == 1
    assert payload["telemetry"]["succeeded"] == 1
    assert "secret-token" not in output
    assert provider.closed
    assert configs[0] == HonchoConfig(
        base_url="https://api.honcho.dev",
        workspace_id="personal",
        peer_id="agent",
        token="secret-token",
        session_prefix="brain",
    )


def test_memory_sync_requires_a_token_without_revealing_its_value(
    tmp_path, monkeypatch, capsys
) -> None:
    monkeypatch.setenv("MISSING_TOKEN", "")
    result = cli.main(
        [
            "--provider",
            "hindsight",
            "--outbox",
            str(tmp_path / "memory.sqlite3"),
            "--token-env",
            "MISSING_TOKEN",
        ]
    )
    assert result == 1
    assert "MISSING_TOKEN is not set" in capsys.readouterr().err


def test_memory_sync_requires_explicit_honcho_append_ack(tmp_path, monkeypatch, capsys) -> None:
    monkeypatch.setenv("HONCHO_TOKEN", "secret-token")
    result = cli.main(
        [
            "--provider",
            "honcho",
            "--outbox",
            str(tmp_path / "memory.sqlite3"),
            "--token-env",
            "HONCHO_TOKEN",
        ]
    )
    assert result == 1
    assert "--allow-append-only" in capsys.readouterr().err


def test_memory_sync_rejects_invalid_token_environment_names(monkeypatch) -> None:
    monkeypatch.setenv("bad-name", "secret")
    with pytest.raises(cli.MemoryArgumentError):
        cli._token_from_env("bad-name")
