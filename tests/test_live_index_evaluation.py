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
        if path == "/v1/context":
            return Response(
                {
                    "context": "private context must never appear in the report",
                    "evidence": [{"source_id": "work-release", "source": "runbooks"}],
                    "metrics": {
                        "retrieved": 2,
                        "included": 1,
                        "omitted": 1,
                        "memories_retrieved": 0,
                        "memories_included": 0,
                        "memories_omitted": 0,
                        "estimated_tokens": 200,
                        "max_tokens": 256,
                    },
                    "retrieval_mode": "lexical-fallback",
                }
            )
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
        "version": 2,
        "release_version": "0.56.9",
        "corpus": {
            "id": "approved-fixture-corpus",
            "revision": "2026-08-25",
            "digest": "sha256:" + "1" * 64,
            "storage": "encrypted-local",
            "approved_at": "2026-08-25T00:00:00Z",
            "expires_at": "2027-08-25T00:00:00Z",
            "reviewer": "test-reviewer",
        },
        "governance": {
            "contract_version": "cortana.approved-corpus.v1",
            "operator_controlled": True,
            "raw_data_external": True,
            "credentials_external": True,
            "private_paths_external": True,
            "scope": {
                "workspaces": ["work"],
                "sources": ["runbooks"],
                "forbidden_sources": ["personal"],
                "memory": "excluded",
            },
            "storage": {"mode": "encrypted-local", "credentials_external": True},
            "reviewer_access": {
                "mode": "local-only",
                "reviewers": ["test-reviewer"],
                "approval_required": True,
            },
            "lifecycle": {
                "retention_days": 90,
                "deletion": "operator-confirmed",
                "redaction": "operator-controlled",
                "incident": "stop-revoke-notify",
            },
            "resource_bounds": {
                "max_request_seconds": 30,
                "max_total_seconds": 300,
                "max_response_bytes": 4 * 1024 * 1024,
                "max_memory_mb": 1024,
                "max_cases": 100,
            },
            "coverage": [{"workspace": "work", "source": "runbooks", "minimum_cases": 0}],
            "provider_synthesis_enabled": True,
        },
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
                "id": "retrieval-release-runbook",
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
                "id": "answer-release-runbook",
                "mode": "provider-synthesis",
                "query": "is the release verified?",
                "project": "work",
                "source": "runbooks",
                "expected_source_ids": ["work-release"],
                "forbidden_source_ids": ["personal-secret"],
                "required_answer_terms": ["release is verified"],
            }
        ],
    }


def test_live_evaluation_measures_retrieval_answer_citations_and_cache() -> None:
    client = Client()
    report = live.evaluate_manifest(
        live.validate_manifest(manifest()), client, require_synthesis=True
    )

    assert report["passed"] is True
    assert report["evaluation"] == "cortana-live-index-v2"
    assert report["provenance"]["release_version"] == "0.56.9"
    assert report["read_only"] is True
    assert report["cache_invalidation_checked"] is False
    assert report["provenance"]["corpus"]["id"] == "approved-fixture-corpus"
    assert report["provenance"]["corpus"]["revision"] == "2026-08-25"
    assert "release verification" not in json.dumps(report["provenance"])
    assert report["metrics"]["recall_at_k"] == 1.0
    assert report["metrics"]["mrr"] == 1.0
    assert report["metrics"]["cache_hit_rate"] == 1.0
    assert report["metrics"]["retrieval_fallback_rate"] == 0.0
    assert report["metrics"]["provider_fallback_rate"] == 0.0
    assert report["retrieval_cases"][0]["retrieval_mode"] == "hybrid"
    assert report["answer_cases"][0]["citations_valid"] is True
    assert report["answer_cases"][0]["answer_terms_valid"] is True
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


def test_live_manifest_rejects_a_stale_release_version() -> None:
    invalid = manifest()
    invalid["release_version"] = "0.56.2"

    with pytest.raises(live.ManifestError, match="release version"):
        live.validate_manifest(invalid, expected_version="0.56.3")


def test_live_manifest_supports_an_explicit_historical_release_override() -> None:
    historical = manifest()
    historical["release_version"] = "0.39.0"

    checked = live.validate_manifest(historical, expected_version="0.39.0")

    assert checked["release_version"] == "0.39.0"


def test_live_cli_accepts_an_explicit_historical_release_override(tmp_path, capsys) -> None:
    historical = manifest()
    historical["release_version"] = "0.39.0"
    path = tmp_path / "historical-manifest.json"
    path.write_text(json.dumps(historical), encoding="utf-8")

    assert live.main([str(path), "--validate-only", "--version", "0.39.0"]) == 0

    report = json.loads(capsys.readouterr().out)
    assert report["provenance"]["release_version"] == "0.39.0"


def test_live_manifest_preflight_is_sanitized_and_does_not_contact_an_index(tmp_path) -> None:
    path = tmp_path / "approved-manifest.json"
    path.write_text(json.dumps(manifest()), encoding="utf-8")

    report = live.preflight_report(live.load_manifest(path))

    assert report["preflight"] == "passed"
    assert report["read_only"] is True
    assert report["index_contacted"] is False
    assert report["case_counts"] == {"retrieval": 1, "context": 0, "answer": 1, "total": 2}
    assert report["provenance"]["corpus"]["id"] == "approved-fixture-corpus"
    assert report["provenance"]["release_version"] == "0.56.9"
    serialized = json.dumps(report)
    assert "release verification" not in serialized
    assert "work-release" not in serialized
    assert "test-reviewer" not in serialized


def test_manifest_rejects_unsafe_or_expired_corpus_metadata() -> None:
    invalid = manifest()
    invalid["corpus"]["digest"] = "sha256:not-a-digest"
    with pytest.raises(live.ManifestError, match="corpus.digest"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["corpus"]["approved_at"] = "2099-01-01T00:00:00Z"
    invalid["corpus"]["expires_at"] = "2100-01-01T00:00:00Z"
    with pytest.raises(live.ManifestError, match="approval window"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["corpus"]["approved_at"] = "2020-01-01T00:00:00Z"
    invalid["corpus"]["expires_at"] = "2021-01-01T00:00:00Z"
    with pytest.raises(live.ManifestError, match="approval window"):
        live.validate_manifest(invalid)


def test_manifest_rejects_governance_scope_coverage_and_provider_without_opt_in() -> None:
    invalid = manifest()
    invalid["governance"]["scope"]["sources"] = ["notes"]
    with pytest.raises(live.ManifestError, match="one configured source"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["governance"]["coverage"][0]["minimum_cases"] = 3
    with pytest.raises(live.ManifestError, match="coverage is incomplete"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["governance"]["provider_synthesis_enabled"] = False
    with pytest.raises(live.ManifestError, match="explicit governance opt-in"):
        live.validate_manifest(invalid)


def test_manifest_rejects_private_paths_and_unsafe_lifecycle() -> None:
    invalid = manifest()
    invalid["governance"]["scope"]["sources"] = ["/Users/private/source"]
    with pytest.raises(live.ManifestError, match="filesystem paths"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["governance"]["lifecycle"]["deletion"] = "automatic"
    with pytest.raises(live.ManifestError, match="lifecycle.deletion"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["governance"]["resource_bounds"]["max_cases"] = 1
    with pytest.raises(live.ManifestError, match="max_cases"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["corpus"]["id"] = "/private/path"
    with pytest.raises(live.ManifestError, match="path separator"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["corpus"]["expires_at"] = "2026-01-01T00:00:00Z"
    with pytest.raises(live.ManifestError, match="timestamps"):
        live.validate_manifest(invalid)


def test_manifest_requires_corpus_provenance_and_reviewer_approval() -> None:
    invalid = manifest()
    invalid.pop("corpus")
    with pytest.raises(live.ManifestError, match="corpus"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["corpus"].pop("reviewer")
    with pytest.raises(live.ManifestError, match="corpus.reviewer"):
        live.validate_manifest(invalid)

    invalid = manifest()
    invalid["corpus"]["reviewer"] = "unlisted-reviewer"
    with pytest.raises(live.ManifestError, match="authorized reviewer"):
        live.validate_manifest(invalid)


def test_checked_in_live_manifest_example_is_valid() -> None:
    checked = live.load_manifest(ROOT / "eval/live-manifest.example.json")
    assert checked["version"] == 2
    assert checked["release_version"] == "0.56.9"
    assert checked["manifest_digest"].startswith("sha256:")
    assert checked["corpus"]["storage"] == "encrypted-local"
    assert len(checked["retrieval_cases"]) == 1
    assert len(checked["answer_cases"]) == 1


def test_live_cli_rejects_remote_http_and_embedded_url_credentials() -> None:
    example = str(ROOT / "eval/live-manifest.example.json")
    with pytest.raises(SystemExit, match="HTTPS"):
        live.main([example, "--base-url", "http://example.test"])
    with pytest.raises(SystemExit, match="embedded credentials"):
        live.main([example, "--base-url", "https://user:secret@example.test"])

    invalid = manifest()
    invalid["version"] = 3
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


def test_live_answer_fails_closed_when_required_terms_are_missing() -> None:
    answer_manifest = manifest()
    answer_manifest["retrieval_cases"] = []
    answer_manifest["answer_cases"][0]["required_answer_terms"] = [
        "a phrase the provider did not answer"
    ]
    report = live.evaluate_manifest(live.validate_manifest(answer_manifest), Client())

    assert report["passed"] is False
    assert report["answer_cases"][0]["answer_terms_valid"] is False
    assert report["answer_cases"][0]["answer_terms_checked"] == 1
    assert report["answer_cases"][0]["answer_terms_missing"] == 1
    assert "a phrase the provider did not answer" not in json.dumps(report)


def test_context_baseline_records_token_bounds_fallback_and_latency_metrics() -> None:
    context_manifest = manifest()
    context_manifest["retrieval_cases"] = []
    context_manifest["answer_cases"] = []
    context_manifest["context_cases"] = [
        {
            "name": "bounded-work-context",
            "query": "private query omitted from reports",
            "project": "work",
            "source": "runbooks",
            "top_k": 5,
            "max_tokens": 256,
            "expected_source_ids": ["work-release"],
            "forbidden_source_ids": ["personal-secret"],
        }
    ]

    report = live.evaluate_manifest(live.validate_manifest(context_manifest), Client())

    assert report["passed"] is True
    assert report["metrics"]["context_pass_rate"] == 1.0
    assert report["metrics"]["retrieval_fallback_rate"] == 1.0
    assert report["metrics"]["token_inclusion_rate"] == 0.5
    assert report["metrics"]["token_omission_rate"] == 0.5
    assert report["metrics"]["token_budget_compliance_rate"] == 1.0
    assert report["metrics"]["latency_ms_p50"] >= 0
    assert report["context_cases"][0]["retrieval_mode"] == "lexical-fallback"
    assert report["context_cases"][0]["token_budget_valid"] is True
    assert "private query omitted" not in json.dumps(report)
    assert "private context must never" not in json.dumps(report)


def test_scope_and_citation_metrics_fail_closed_for_forbidden_evidence() -> None:
    class LeakingClient(Client):
        def post(self, path: str, *, json: dict, timeout: float) -> Response:
            self.calls.append((path, json))
            if path == "/v1/search":
                return Response(
                    [
                        {
                            "source_id": "personal-secret",
                            "source": "personal",
                            "project": "personal",
                        }
                    ]
                )
            return Response(
                {
                    "answer": "Do not expose this. [1]",
                    "evidence": [
                        {
                            "source_id": "personal-secret",
                            "source": "personal",
                            "project": "personal",
                        }
                    ],
                    "mode": "extractive",
                    "cached": False,
                }
            )

    leaking_manifest = manifest()
    leaking_manifest["retrieval_cases"][0]["forbidden_projects"] = ["personal"]
    leaking_manifest["retrieval_cases"][0]["forbidden_sources"] = ["personal"]
    leaking_manifest["answer_cases"][0]["forbidden_projects"] = ["personal"]
    leaking_manifest["answer_cases"][0]["forbidden_sources"] = ["personal"]
    report = live.evaluate_manifest(live.validate_manifest(leaking_manifest), LeakingClient())

    assert report["passed"] is False
    assert report["metrics"]["forbidden_source_leak_count"] == 6
    assert report["metrics"]["invalid_citation_count"] == 0
    assert report["retrieval_cases"][0]["project_scope_valid"] is False
    assert report["answer_cases"][0]["forbidden_project_leaks"] == ["personal"]
