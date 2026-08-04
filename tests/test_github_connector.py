from __future__ import annotations

import base64
import json
import os
import stat
from pathlib import Path

import httpx
import pytest

from cortana.connectors import github


def _response(request: httpx.Request, payload: object, status: int = 200) -> httpx.Response:
    return httpx.Response(status, json=payload, request=request)


def _client(handler):
    return httpx.Client(
        base_url="https://api.github.com",
        transport=httpx.MockTransport(handler),
    )


def test_github_fetches_only_selected_text_blobs(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GITHUB_TEST_TOKEN", "secret")
    blob = base64.b64encode(b"print('hello')\n").decode()

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/repos/acme/project":
            return _response(
                request,
                {"default_branch": "main", "pushed_at": "2026-08-01T12:00:00Z"},
            )
        if request.url.path.endswith("/git/trees/main"):
            return _response(
                request,
                {
                    "truncated": False,
                    "tree": [
                        {"type": "blob", "path": "src/main.py", "sha": "abc", "size": len(blob)},
                        {"type": "blob", "path": "target/generated.rs", "sha": "skip", "size": 10},
                        {"type": "tree", "path": "src", "sha": "tree"},
                    ],
                },
            )
        if request.url.path.endswith("/git/blobs/abc"):
            return _response(request, {"encoding": "base64", "content": blob})
        raise AssertionError(f"unexpected GitHub request: {request.url}")

    documents = list(
        github.fetch(
            ["acme/project"],
            "work",
            token_env="GITHUB_TEST_TOKEN",
            client=_client(handler),
        )
    )

    assert len(documents) == 1
    assert documents[0].source == "github"
    assert documents[0].source_id == "acme/project:src/main.py:abc"
    assert documents[0].project == "work"
    assert documents[0].metadata["repository"] == "acme/project"
    assert documents[0].uri == "https://github.com/acme/project/blob/main/src/main.py"


def test_github_reads_owner_only_access_token_file(tmp_path: Path) -> None:
    token_path = tmp_path / "github-token.json"
    token_path.write_text(json.dumps({"access_token": "secret", "token_type": "bearer"}))
    if os.name == "posix":
        token_path.chmod(stat.S_IRUSR | stat.S_IWUSR)

    def handler(request: httpx.Request) -> httpx.Response:
        assert request.headers["authorization"] == "Bearer secret"
        if request.url.path == "/repos/acme/project":
            return _response(request, {"default_branch": "main"})
        if request.url.path.endswith("/git/trees/main"):
            return _response(request, {"truncated": False, "tree": []})
        raise AssertionError(request.url)

    assert (
        list(github.fetch(["acme/project"], "work", token_path=token_path, client=_client(handler)))
        == []
    )


@pytest.mark.parametrize("token", ["secret\x7fvalue", "secret\u0085value"])
def test_github_rejects_control_characters_in_environment_token(
    monkeypatch: pytest.MonkeyPatch, token: str
) -> None:
    monkeypatch.setenv("GITHUB_TEST_TOKEN", token)

    with pytest.raises(RuntimeError, match="environment value is invalid"):
        github._access_token(None, "GITHUB_TEST_TOKEN")


def test_github_rejects_nul_in_access_token_file(tmp_path: Path) -> None:
    token_path = tmp_path / "github-token.json"
    token_path.write_text(json.dumps({"access_token": "secret\x00value"}))
    if os.name == "posix":
        token_path.chmod(stat.S_IRUSR | stat.S_IWUSR)

    with pytest.raises(RuntimeError, match="file contains an invalid access token"):
        github._access_token(token_path, "GITHUB_TEST_TOKEN")


def test_github_rejects_truncated_trees(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GITHUB_TEST_TOKEN", "secret")

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/repos/acme/project":
            return _response(request, {"default_branch": "main"})
        return _response(request, {"truncated": True, "tree": []})

    with pytest.raises(RuntimeError, match="tree is truncated"):
        list(
            github.fetch(
                ["acme/project"],
                "work",
                token_env="GITHUB_TEST_TOKEN",
                client=_client(handler),
            )
        )


def test_github_bounds_documents_before_fetching_later_repositories(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("GITHUB_TEST_TOKEN", "secret")
    requests: list[str] = []
    blob = base64.b64encode(b"one\n").decode()

    def handler(request: httpx.Request) -> httpx.Response:
        requests.append(request.url.path)
        if request.url.path.endswith("/git/trees/main"):
            return _response(
                request,
                {
                    "truncated": False,
                    "tree": [{"type": "blob", "path": "one.py", "sha": "one", "size": 4}],
                },
            )
        if request.url.path.endswith("/git/blobs/one"):
            return _response(request, {"encoding": "base64", "content": blob})
        if request.url.path.startswith("/repos/"):
            return _response(request, {"default_branch": "main"})
        raise AssertionError(request.url)

    documents = list(
        github.fetch(
            ["acme/one", "acme/two"],
            "work",
            token_env="GITHUB_TEST_TOKEN",
            max_documents=1,
            client=_client(handler),
        )
    )

    assert len(documents) == 1
    assert not any(path == "/repos/acme/two" for path in requests)


@pytest.mark.parametrize("value", ["acme", "acme/project/path", "https://github.com/acme/project"])
def test_github_requires_owner_repository_identifier(
    monkeypatch: pytest.MonkeyPatch, value: str
) -> None:
    monkeypatch.setenv("GITHUB_TEST_TOKEN", "secret")
    with pytest.raises(RuntimeError, match="owner/name"):
        list(
            github.fetch(
                [value], "work", token_env="GITHUB_TEST_TOKEN", client=_client(lambda *_: None)
            )
        )
