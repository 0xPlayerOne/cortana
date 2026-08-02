from __future__ import annotations

from dataclasses import dataclass
from typing import Any
from urllib.parse import quote_plus

import httpx

from .models import MemoryArgumentError, MemoryDocument
from .provider import MemoryProvider, ProviderError


@dataclass(frozen=True)
class HindsightConfig:
    base_url: str
    bank: str
    token: str


class HindsightHttpProvider(MemoryProvider):
    """HTTP provider for Hindsight.

    The HTTP token is intentionally never logged or exposed in diagnostics.
    """

    def __init__(
        self,
        config: HindsightConfig,
        *,
        timeout_seconds: float = 5.0,
        client: httpx.Client | None = None,
    ) -> None:
        self._config = config
        self._base = self._safe_base_url(self._config.base_url)
        self._client = client or self._build_client(timeout_seconds)
        self._owns_client = client is None

        if not self._config.bank or not self._config.bank.strip():
            raise MemoryArgumentError("bank must be configured")
        if any(character in self._config.bank for character in "?#/\\\r\n"):
            raise MemoryArgumentError("bank contains unsafe URL characters")
        if not self._config.token or not self._config.token.strip():
            raise MemoryArgumentError("token must be configured")
        if any(character in self._config.token for character in "\r\n"):
            raise MemoryArgumentError("token contains unsafe characters")

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

    def _build_client(self, timeout_seconds: float) -> httpx.Client:
        if timeout_seconds <= 0:
            raise MemoryArgumentError("timeout_seconds must be positive")
        return httpx.Client(
            base_url=str(self._base),
            timeout=timeout_seconds,
            follow_redirects=False,
        )

    @property
    def configured(self) -> bool:
        return bool(self._config.base_url and self._config.bank and self._config.token)

    def retain(self, document: MemoryDocument) -> None:
        response = self._request(
            "POST",
            self._url(f"v1/default/banks/{quote_plus(self._config.bank)}/memories/retain"),
            json=document.retention_payload(),
        )
        if response.status_code < 200 or response.status_code >= 300:
            raise ProviderError(
                f"retain failed: HTTP {response.status_code}",
                retriable=500 <= response.status_code < 600 or response.status_code == 429,
            )

    def delete(self, document_id: str) -> None:
        if not document_id or any(character in document_id for character in "?#/\\\r\n"):
            raise MemoryArgumentError("document_id contains unsafe URL characters")
        response = self._request(
            "DELETE",
            self._url(
                f"v1/default/banks/{quote_plus(self._config.bank)}/documents/{quote_plus(document_id)}"
            ),
        )
        if response.status_code < 200 or response.status_code >= 300:
            raise ProviderError(
                f"delete failed: HTTP {response.status_code}",
                retriable=500 <= response.status_code < 600 or response.status_code == 429,
            )

    def _url(self, relative_path: str) -> str:
        # Keep configured base URL path handling while accepting explicit relative paths.
        return f"{str(self._base).rstrip('/')}/{relative_path.lstrip('/')}"

    def _request(self, method: str, url: str, **kwargs: Any) -> httpx.Response:
        headers = {"Authorization": f"Bearer {self._config.token}"}
        try:
            return self._client.request(method, url, headers=headers, **kwargs)
        except httpx.RequestError as error:
            raise ProviderError("request failed", retriable=True) from error

    def diagnostics(self) -> dict[str, object]:
        """Return provider diagnostics without exposing secret material."""

        return {
            "configured": self.configured,
            "base_url": str(self._base),
            "bank": self._config.bank,
            "has_token": bool(self._config.token),
            "configured_http": self._base.scheme.startswith("http"),
        }

    def close(self) -> None:
        if self._owns_client:
            self._client.close()


__all__ = ["HindsightConfig", "HindsightHttpProvider"]
