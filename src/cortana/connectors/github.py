from __future__ import annotations

import base64
import datetime as dt
import os
import re
import time
from collections.abc import Iterable
from typing import Any
from urllib.parse import quote

import httpx

from .http import json_payload
from .model import Document

_REPOSITORY = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
_TEXT_SUFFIXES = {
    ".c",
    ".cc",
    ".cfg",
    ".cpp",
    ".cs",
    ".css",
    ".dart",
    ".ex",
    ".exs",
    ".go",
    ".h",
    ".hpp",
    ".html",
    ".ini",
    ".java",
    ".js",
    ".json",
    ".jsx",
    ".kt",
    ".kts",
    ".less",
    ".md",
    ".mjs",
    ".php",
    ".py",
    ".rb",
    ".rs",
    ".sass",
    ".scss",
    ".sh",
    ".sql",
    ".swift",
    ".toml",
    ".ts",
    ".tsx",
    ".vue",
    ".xml",
    ".yaml",
    ".yml",
    ".zsh",
}
_TEXT_NAMES = {"Dockerfile", "Makefile", "Procfile", "LICENSE", "NOTICE"}
_EXCLUDED_PARTS = {
    ".git",
    ".github",
    "__pycache__",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
    "vendor",
}
MAX_BLOB_BYTES = 512 * 1024
MAX_TREE_ENTRIES = 100_000
MAX_REPOSITORIES = 32


def fetch(
    repositories: list[str],
    project: str,
    token_env: str = "GITHUB_TOKEN",
    max_documents: int | None = None,
    client: httpx.Client | None = None,
) -> Iterable[Document]:
    """Fetch bounded, text-only snapshots from explicitly selected repositories.

    Repository selection is intentionally an allowlist. The connector never
    searches a user's account or follows links supplied by repository content.
    A complete snapshot fails closed when GitHub reports a truncated tree.
    """

    normalized = _normalize_repositories(repositories)
    if not normalized:
        raise RuntimeError("GitHub source requires at least one repository")
    if len(normalized) > MAX_REPOSITORIES:
        raise RuntimeError(f"GitHub source supports at most {MAX_REPOSITORIES} repositories")
    if max_documents is not None and max_documents <= 0:
        raise ValueError("max_documents must be greater than zero")
    token = os.environ.get(token_env, "").strip()
    if not token:
        raise RuntimeError(f"GitHub token environment variable is not configured: {token_env}")

    owns_client = client is None
    session = client or httpx.Client(
        base_url="https://api.github.com",
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
        },
        timeout=60,
        follow_redirects=False,
    )
    try:
        emitted = 0
        for repository in normalized:
            if max_documents is not None and emitted >= max_documents:
                return
            metadata = _get_json(session, f"/repos/{repository}")
            if not isinstance(metadata, dict):
                raise RuntimeError(f"GitHub repository metadata is invalid: {repository}")
            branch = str(metadata.get("default_branch") or "").strip()
            if not branch or not _safe_ref(branch):
                raise RuntimeError(f"GitHub repository has no safe default branch: {repository}")
            updated_at = _parse_timestamp(metadata.get("pushed_at") or metadata.get("updated_at"))
            tree = _get_json(session, f"/repos/{repository}/git/trees/{quote(branch, safe='')}")
            if not isinstance(tree, dict):
                raise RuntimeError(f"GitHub repository tree is invalid: {repository}")
            entries = tree.get("tree")
            if not isinstance(entries, list):
                raise RuntimeError(
                    f"GitHub repository tree has an invalid entry list: {repository}"
                )
            if len(entries) > MAX_TREE_ENTRIES:
                raise RuntimeError(f"GitHub repository tree is too large: {repository}")
            if tree.get("truncated"):
                raise RuntimeError(f"GitHub repository tree is truncated: {repository}")
            for entry in entries:
                if max_documents is not None and emitted >= max_documents:
                    return
                if not isinstance(entry, dict) or entry.get("type") != "blob":
                    continue
                path = str(entry.get("path") or "")
                blob_sha = str(entry.get("sha") or "")
                size = _positive_int(entry.get("size"))
                if (
                    not _is_indexable_path(path)
                    or not blob_sha
                    or size is None
                    or size > MAX_BLOB_BYTES
                ):
                    continue
                payload = _get_json(
                    session, f"/repos/{repository}/git/blobs/{quote(blob_sha, safe='')}"
                )
                content = _decode_blob(payload, repository, path)
                if not content.strip():
                    continue
                owner, repo = repository.split("/", 1)
                yield Document(
                    source="github",
                    source_id=f"{repository}:{path}:{blob_sha}",
                    title=f"{repository}: {path}",
                    content=content,
                    uri=f"https://github.com/{owner}/{repo}/blob/{quote(branch, safe='')}/{quote(path, safe='/')}",
                    updated_at=updated_at,
                    project=project,
                    metadata={
                        "repository": repository,
                        "path": path,
                        "blob_sha": blob_sha,
                        "default_branch": branch,
                    },
                )
                emitted += 1
    finally:
        if owns_client:
            session.close()


def _normalize_repositories(repositories: list[str]) -> list[str]:
    result: list[str] = []
    for value in repositories:
        repository = value.strip().strip("/")
        if not _REPOSITORY.fullmatch(repository):
            raise RuntimeError(f"GitHub repository must use owner/name form: {value}")
        normalized = repository.lower()
        if normalized not in result:
            result.append(normalized)
    return result


def _get_json(client: httpx.Client, path: str) -> Any:
    for attempt in range(4):
        response = client.get(path)
        if response.status_code in {429, 500, 502, 503, 504} and attempt < 3:
            delay = _retry_after(response, attempt)
            time.sleep(delay)
            continue
        try:
            response.raise_for_status()
        except httpx.HTTPStatusError as error:
            raise RuntimeError(f"GitHub API request failed ({response.status_code})") from error
        return json_payload(response)
    raise RuntimeError("GitHub API request retry budget exhausted")


def _retry_after(response: httpx.Response, attempt: int) -> float:
    header = response.headers.get("retry-after")
    try:
        if header is not None:
            return min(30.0, max(0.1, float(header)))
    except ValueError:
        pass
    return min(30.0, 0.5 * (2**attempt))


def _decode_blob(payload: Any, repository: str, path: str) -> str:
    if not isinstance(payload, dict) or payload.get("encoding") != "base64":
        raise RuntimeError(f"GitHub blob has an unsupported encoding: {repository}:{path}")
    encoded = payload.get("content")
    if not isinstance(encoded, str):
        raise RuntimeError(f"GitHub blob content is missing: {repository}:{path}")
    try:
        raw = base64.b64decode(encoded, validate=False)
        content = raw.decode("utf-8")
    except (ValueError, UnicodeDecodeError) as error:
        raise RuntimeError(f"GitHub blob is not UTF-8 text: {repository}:{path}") from error
    if "\x00" in content:
        raise RuntimeError(f"GitHub blob contains binary content: {repository}:{path}")
    return content


def _is_indexable_path(path: str) -> bool:
    if not path or path.startswith("/") or "\\" in path:
        return False
    parts = path.split("/")
    if any(not part or part in {".", ".."} or part in _EXCLUDED_PARTS for part in parts):
        return False
    name = parts[-1]
    return name in _TEXT_NAMES or any(name.lower().endswith(suffix) for suffix in _TEXT_SUFFIXES)


def _safe_ref(value: str) -> bool:
    return bool(value) and ".." not in value and "\x00" not in value and "\\" not in value


def _positive_int(value: Any) -> int | None:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed >= 0 else None


def _parse_timestamp(value: Any) -> dt.datetime:
    if isinstance(value, str):
        try:
            parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
            return parsed.astimezone(dt.UTC)
        except ValueError:
            pass
    return dt.datetime.now(dt.UTC)
