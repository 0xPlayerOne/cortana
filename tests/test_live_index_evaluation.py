import importlib.util
import json
from contextlib import contextmanager
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "cortana_live_index_evaluation", ROOT / "scripts/evaluate-live-index.py"
)
assert SPEC and SPEC.loader
live = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(live)


class Response:
    def __init__(self, payload, status_code: int = 200) -> None:
        self._payload = payload
        self.status_code = status_code
        self.headers: dict[str, str] = {}

    def json(self):
        return self._payload

    @property
    def content(self) -> bytes:
        return json.dumps(self._payload).encode()

    def iter_bytes(self):
        yield self.content


class Client:
    def __init__(self) -> None:
        self.calls: list[tuple[str, dict]] = []

    def post(self, path: str, *, json: dict, timeout: float) -> Response:
        assert timeout > 0
        self.calls.append((path, json))
        if path == "/v1/search":
            response = Response(
                [
                    {"source_id": "work-release", "source": "runbooks"},
                    {"source_id": "work-old", "source": "runbooks"},
                ]
            )
            response.headers = {
                "x-cortana-retrieval-mode": "hybrid",
                "x-cortana-retrieval-degraded": "false",
            }
            return response
        if len([path for path, _ in self.calls if path == "/v1/answer"]) == 1:
            return Response(
                {
                    "answer": "The release is verified. [1]",
                    "evidence": [{"source_id": "work-release", "source": "runbooks"}],
                    "mode": "synthesized",
                    "cached": False,
                }
            )
        return Response(
            {
                "answer": "The release is verified. [1]",
                "evidence": [{"source_id": "work-release", "source": "runbooks"}],
                "mode": "synthesized",
                "cached": True,
            }
        )

    @contextmanager
    def stream(self, _method: str, path: str, *, json: dict, timeout: float):
        yield self.post(path, json=json, timeout=timeout)


def manifest() -> dict:
    return {
        "version": 1,
        "thresholds": {
            "min_recall_at_k": 1.0,
            "min_mrr": 1.0,
            "min_retrieval_pass_rate": 1.0,
            "min_answer_pass_rate": 1.0,
            "min_citation_validity": 1.0,
            "max_latency_ms": 60_000,
        },
        "retrieval_cases": [
            {
                "name": "release-runbook",
                "query": "release verification",
                "project": "work",
                "source": "runbooks",
                "top_k": 5,
                "expected_source_ids": ["work-release"],
                "forbidden_source_ids": ["personal-secret"],
            }
        ],
        "answer_cases": [
            {
                "name": "release-answer",
                "query": "is the release verified?",
                "project": "work",
                "source": "runbooks",
                "expected_source_ids": ["work-release"],
                "forbidden_source_ids": ["personal-secret"],
            }
        ],
    }


def test_live_evaluation_measures_retrieval_answer_citations_and_cache() -> None:
    client = Client()
    report = live.evaluate_manifest(
        live.validate_manifest(manifest()), client, require_synthesis=True
    )

    assert report["passed"] is True
    assert report["read_only"] is True
    assert report["cache_invalidation_checked"] is False
    assert report["metrics"]["recall_at_k"] == 1.0
    assert report["metrics"]["mrr"] == 1.0
    assert report["metrics"]["cache_hit_rate"] == 1.0
    assert report["metrics"]["retrieval_fallback_rate"] == 0.0
    assert report["metrics"]["provider_fallback_rate"] == 0.0
    assert report["retrieval_cases"][0]["retrieval_mode"] == "hybrid"
    assert report["answer_cases"][0]["citations_valid"] is True
    assert len(client.calls) == 3
    assert "limit" in client.calls[0][1]
    assert "limit" not in client.calls[1][1]
    serialized = json.dumps(report)
    assert "release verification" not in serialized
    assert "The release is verified" not in serialized


def test_live_manifest_rejects_unsafe_query_and_unknown_version() -> None:
    invalid = manifest()
    invalid["answer_cases"][0]["query"] = "x" * (live.MAX_QUERY_BYTES + 1)
    with pytest.raises(live.ManifestError):
        live.validate_manifest(invalid)


def test_checked_in_live_manifest_example_is_valid() -> None:
    checked = live.load_manifest(ROOT / "eval/live-manifest.example.json")
    assert checked["version"] == 1
    assert len(checked["retrieval_cases"]) == 1
    assert len(checked["answer_cases"]) == 1


def test_live_cli_rejects_remote_http_and_embedded_url_credentials() -> None:
    example = str(ROOT / "eval/live-manifest.example.json")
    with pytest.raises(SystemExit, match="HTTPS"):
        live.main([example, "--base-url", "http://example.test"])
    with pytest.raises(SystemExit, match="embedded credentials"):
        live.main([example, "--base-url", "https://user:secret@example.test"])

    invalid = manifest()
    invalid["version"] = 2
    with pytest.raises(live.ManifestError):
        live.validate_manifest(invalid)


def test_live_evaluation_fails_closed_on_http_errors_without_body_leak() -> None:
    class FailingClient:
        @contextmanager
        def stream(self, _method: str, _path: str, *, json: dict, timeout: float):
            del json
            assert timeout > 0
            yield Response({"error": "private provider response"}, status_code=503)

    report = live.evaluate_manifest(live.validate_manifest(manifest()), FailingClient())
    assert report["passed"] is False
    assert report["retrieval_cases"][0]["error_status"] == 503
    assert report["answer_cases"][0]["error_status"] == 503
    assert "private provider response" not in json.dumps(report)


def test_live_answer_fails_closed_when_evidence_ignores_source_scope() -> None:
    class WrongSourceClient:
        @contextmanager
        def stream(self, _method: str, path: str, *, json: dict, timeout: float):
            del json
            assert timeout > 0
            if path == "/v1/search":
                yield Response([])
                return
            yield Response(
                {
                    "answer": "The answer cites the wrong source. [1]",
                    "evidence": [{"source_id": "personal-secret", "source": "personal"}],
                    "mode": "synthesized",
                    "cached": False,
                }
            )

    answer_manifest = manifest()
    answer_manifest["retrieval_cases"] = []
    report = live.evaluate_manifest(
        live.validate_manifest(answer_manifest), WrongSourceClient(), require_synthesis=True
    )

    assert report["passed"] is False
    assert report["answer_cases"][0]["source_scope_valid"] is False
