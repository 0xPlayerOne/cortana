from __future__ import annotations

from dataclasses import dataclass
from typing import Any
from urllib.parse import quote

import httpx

from .models import MemoryArgumentError, MemoryDocument
from .provider import MemoryProvider, ProviderError

_IDENTIFIER_LIMIT = 128
_MAX_MESSAGE_CHARS = 128_000
_DOCUMENT_ID_LENGTH = 64


@dataclass(frozen=True)
class HonchoConfig:
    """Connection and namespace settings for the optional Honcho sidecar."""

    base_url: str
    workspace_id: str
    peer_id: str
    token: str
    session_prefix: str = "cortana"


class HonchoHttpProvider(MemoryProvider):
    """HTTP provider for Honcho's v3 session/message API.

    Honcho is a derived, opt-in sidecar rather than Cortana's source of truth. Each retained
    document gets one deterministic session, which makes the provider's session-level delete
    endpoint safe to use for a single canonical document. The adapter never participates in
    normal ingestion unless an operator explicitly drains the memory outbox with it.
    """

    def __init__(
        self,
        config: HonchoConfig,
        *,
        timeout_seconds: float = 5.0,
        client: httpx.Client | None = None,
    ) -> None:
        self._config = config
        self._base = self._safe_base_url(config.base_url)
        self._validate_identifier("workspace_id", config.workspace_id)
        self._validate_identifier("peer_id", config.peer_id)
        self._validate_identifier("session_prefix", config.session_prefix)
        if not config.token or not config.token.strip():
            raise MemoryArgumentError("token must be configured")
        if any(character in config.token for character in "\r\n"):
            raise MemoryArgumentError("token contains unsafe characters")

        self._client = client or self._build_client(timeout_seconds)
        self._owns_client = client is None

    @staticmethod
    def _safe_base_url(base_url: str) -> httpx.URL:
        if not base_url.strip():
            raise MemoryArgumentError("base_url cannot be empty")
        try:
            parsed = httpx.URL(base_url)
        except ValueError as error:
            raise MemoryArgumentError(f"invalid base_url: {error}") from error
        if parsed.scheme not in {"http", "https"}:
            raise MemoryArgumentError("base_url must be http(s)")
        if parsed.query or parsed.fragment or parsed.username or parsed.password:
            raise MemoryArgumentError("base_url must not contain query, fragment, or credentials")
        if parsed.scheme == "http" and parsed.host not in {"127.0.0.1", "localhost", "::1"}:
            raise MemoryArgumentError("remote base_url must use HTTPS")
        return parsed

    @staticmethod
    def _validate_identifier(name: str, value: str) -> None:
        if not value or len(value) > _IDENTIFIER_LIMIT:
            raise MemoryArgumentError(f"{name} must be 1-{_IDENTIFIER_LIMIT} characters")
        if any(
            character not in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-"
            for character in value
        ):
            raise MemoryArgumentError(f"{name} contains unsafe URL characters")

    def _build_client(self, timeout_seconds: float) -> httpx.Client:
        if timeout_seconds <= 0:
            raise MemoryArgumentError("timeout_seconds must be positive")
        return httpx.Client(base_url=str(self._base), timeout=timeout_seconds)

    @property
    def configured(self) -> bool:
        return bool(
            self._config.base_url
            and self._config.workspace_id
            and self._config.peer_id
            and self._config.token
        )

    def retain(self, document: MemoryDocument) -> None:
        payload = {
            "messages": [
                {
                    "content": self._message_content(document),
                    "peer_id": self._config.peer_id,
                    "metadata": {
                        "cortana_document_id": document.document_id,
                        "cortana_project": document.project,
                        "cortana_source": document.source,
                        "cortana_source_id": document.source_id,
                        "cortana_tags": document.tags,
                        "source_metadata": document.metadata,
                    },
                }
            ]
        }
        response = self._request("POST", self._messages_url(document.document_id), json=payload)
        self._raise_for_status(response, "retain")

    def delete(self, document_id: str) -> None:
        session_id = self._session_id(document_id)
        response = self._request("DELETE", self._session_url(session_id))
        self._raise_for_status(response, "delete")

    @staticmethod
    def _message_content(document: MemoryDocument) -> str:
        parts = [document.title.strip()]
        if document.context and document.context.strip():
            parts.append(f"Context: {document.context.strip()}")
        parts.append(document.content.strip())
        content = "\n\n".join(parts)
        if len(content) <= _MAX_MESSAGE_CHARS:
            return content
        marker = "\n\n[Content truncated by Cortana for Honcho]\n\n"
        remaining = _MAX_MESSAGE_CHARS - len(marker)
        head = remaining // 2
        tail = remaining - head
        return content[:head] + marker + content[-tail:]

    def _session_id(self, document_id: str) -> str:
        if len(document_id) != _DOCUMENT_ID_LENGTH or any(
            character not in "0123456789abcdef" for character in document_id
        ):
            raise MemoryArgumentError("document_id must be a Cortana stable document id")
        return f"{self._config.session_prefix}-{document_id}"

    def _workspace_url(self, suffix: str) -> str:
        workspace = quote(self._config.workspace_id, safe="")
        return f"{str(self._base).rstrip('/')}/v3/workspaces/{workspace}/{suffix.lstrip('/')}"

    def _session_url(self, session_id: str) -> str:
        return self._workspace_url(f"sessions/{quote(session_id, safe='')}")

    def _messages_url(self, document_id: str) -> str:
        return f"{self._session_url(self._session_id(document_id))}/messages"

    def _request(self, method: str, url: str, **kwargs: Any) -> httpx.Response:
        headers = {"Authorization": f"Bearer {self._config.token}"}
        try:
            return self._client.request(method, url, headers=headers, **kwargs)
        except httpx.RequestError as error:
            raise ProviderError("request failed", retriable=True) from error

    @staticmethod
    def _raise_for_status(response: httpx.Response, operation: str) -> None:
        if 200 <= response.status_code < 300:
            return
        raise ProviderError(
            f"{operation} failed: HTTP {response.status_code}",
            retriable=response.status_code in {408, 429} or response.status_code >= 500,
        )

    def diagnostics(self) -> dict[str, object]:
        """Return provider diagnostics without exposing secret material."""

        return {
            "configured": self.configured,
            "base_url": str(self._base),
            "workspace_id": self._config.workspace_id,
            "peer_id": self._config.peer_id,
            "session_prefix": self._config.session_prefix,
            "has_token": bool(self._config.token),
            "configured_http": self._base.scheme.startswith("http"),
        }

    def close(self) -> None:
        if self._owns_client:
            self._client.close()


__all__ = ["HonchoConfig", "HonchoHttpProvider"]
