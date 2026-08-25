#!/usr/bin/env python3
"""Run a bounded, read-only evaluation against an approved Cortana index.

The manifest contains operator-authored queries and expected source IDs, not
document bodies.  This harness talks only to the query API: it never invokes
ingestion, reconciliation, service management, or backup/restore.  Reports
contain source IDs and bounded metrics, but never echo queries, answers, or
provider error bodies.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import time
from collections.abc import Mapping, Sequence
from datetime import datetime
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

import httpx

MAX_MANIFEST_BYTES = 1 * 1024 * 1024
MAX_CASES = 100
MAX_QUERY_BYTES = 16 * 1024
MAX_ID_BYTES = 512
MAX_ANSWER_TERMS = 32
MAX_ANSWER_TERM_BYTES = 256
MAX_CORPUS_METADATA_BYTES = 256
MAX_RESPONSE_BYTES = 4 * 1024 * 1024
MAX_TOTAL_SECONDS = 300.0
MAX_REQUEST_SECONDS = 60.0
DEFAULT_REQUEST_SECONDS = 30.0
DEFAULT_TOTAL_SECONDS = 300.0
_CITATION = re.compile(r"\[(\d+)\]")
_SHA256_DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")


class ManifestError(ValueError):
    """The operator-supplied manifest is outside the safety contract."""


def _bounded_text(value: Any, label: str, *, required: bool = False) -> str | None:
    if not isinstance(value, str):
        if required:
            raise ManifestError(f"{label} must be a string")
        return None
    if not value.strip() and required:
        raise ManifestError(f"{label} must not be empty")
    if len(value.encode("utf-8")) > MAX_QUERY_BYTES:
        raise ManifestError(f"{label} exceeds the {MAX_QUERY_BYTES} byte limit")
    if any(character in value for character in "\r\n\x00"):
        raise ManifestError(f"{label} contains a control character")
    return value


def _source_ids(value: Any, label: str) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, list) or len(value) > 100:
        raise ManifestError(f"{label} must be a list of at most 100 IDs")
    result: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item.strip():
            raise ManifestError(f"{label} contains an invalid ID")
        if len(item.encode("utf-8")) > MAX_ID_BYTES:
            raise ManifestError(f"{label} contains an oversized ID")
        result.append(item)
    return result


def _answer_terms(value: Any, label: str) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, list) or len(value) > MAX_ANSWER_TERMS:
        raise ManifestError(f"{label} must be a list of at most {MAX_ANSWER_TERMS} terms")
    result: list[str] = []
    seen: set[str] = set()
    for item in value:
        if not isinstance(item, str) or not item.strip():
            raise ManifestError(f"{label} contains an invalid term")
        if len(item.encode("utf-8")) > MAX_ANSWER_TERM_BYTES:
            raise ManifestError(
                f"{label} contains a term over the {MAX_ANSWER_TERM_BYTES} byte limit"
            )
        if any(character in item for character in "\r\n\x00"):
            raise ManifestError(f"{label} contains a control character")
        normalized = item.casefold()
        if normalized not in seen:
            seen.add(normalized)
            result.append(item)
    return result


def _corpus_metadata(value: Any) -> dict[str, str] | None:
    """Validate non-secret provenance for an operator-approved corpus.

    The manifest still keeps raw queries outside the repository.  This small
    metadata block lets a report distinguish a corpus or approval change from
    a product regression without echoing private content or filesystem paths.
    """
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ManifestError("corpus must be an object")

    def metadata_text(name: str, *, required: bool = True) -> str | None:
        result = _bounded_text(value.get(name), f"corpus.{name}", required=required)
        if result is not None and len(result.encode("utf-8")) > MAX_CORPUS_METADATA_BYTES:
            raise ManifestError(f"corpus.{name} exceeds the {MAX_CORPUS_METADATA_BYTES} byte limit")
        if result is not None and any(separator in result for separator in ("/", "\\")):
            raise ManifestError(f"corpus.{name} must not contain a path separator")
        return result

    corpus_id = metadata_text("id")
    revision = metadata_text("revision")
    digest = metadata_text("digest")
    assert corpus_id is not None and revision is not None and digest is not None
    if _SHA256_DIGEST.fullmatch(digest) is None:
        raise ManifestError("corpus.digest must be a sha256:<64 lowercase hex> digest")

    storage = metadata_text("storage")
    if storage not in {"local", "encrypted-local"}:
        raise ManifestError("corpus.storage must be `local` or `encrypted-local`")
    approved_at = metadata_text("approved_at")
    expires_at = metadata_text("expires_at", required=False)
    try:
        approved_time = datetime.fromisoformat(approved_at.replace("Z", "+00:00"))
        if approved_time.tzinfo is None:
            raise ValueError("timezone required")
        expires_time = (
            datetime.fromisoformat(expires_at.replace("Z", "+00:00"))
            if expires_at is not None
            else None
        )
        if expires_time is not None and (
            expires_time.tzinfo is None or expires_time <= approved_time
        ):
            raise ValueError("expiry must be after approval")
    except ValueError as error:
        raise ManifestError(
            "corpus approval timestamps must be RFC3339 with a valid window"
        ) from error

    reviewer = metadata_text("reviewer", required=False)
    result = {
        "id": corpus_id,
        "revision": revision,
        "digest": digest,
        "storage": storage,
        "approved_at": approved_at,
    }
    if expires_at is not None:
        result["expires_at"] = expires_at
    if reviewer is not None:
        result["reviewer"] = reviewer
    return result


def _case(value: Any, label: str, *, answer: bool = False) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ManifestError(f"{label} must be an object")
    name = _bounded_text(value.get("name"), f"{label}.name", required=True)
    query = _bounded_text(value.get("query"), f"{label}.query", required=True)
    project = _bounded_text(value.get("project"), f"{label}.project")
    source = _bounded_text(value.get("source"), f"{label}.source")
    top_k = value.get("top_k", 10)
    if not isinstance(top_k, int) or isinstance(top_k, bool) or not 1 <= top_k <= 50:
        raise ManifestError(f"{label}.top_k must be between 1 and 50")
    expected_source_ids = _source_ids(
        value.get("expected_source_ids"), f"{label}.expected_source_ids"
    )
    if not expected_source_ids:
        raise ManifestError(f"{label}.expected_source_ids must contain at least one ID")
    return {
        "name": name,
        "query": query,
        "project": project,
        "source": source,
        "top_k": top_k,
        "expected_source_ids": expected_source_ids,
        "forbidden_source_ids": _source_ids(
            value.get("forbidden_source_ids"), f"{label}.forbidden_source_ids"
        ),
        "required_answer_terms": _answer_terms(
            value.get("required_answer_terms"), f"{label}.required_answer_terms"
        )
        if answer
        else [],
        "answer": answer,
    }


def validate_manifest(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict) or value.get("version") != 1:
        raise ManifestError("manifest version must be 1")
    retrieval_cases = value.get("retrieval_cases", [])
    answer_cases = value.get("answer_cases", [])
    if not isinstance(retrieval_cases, list) or not isinstance(answer_cases, list):
        raise ManifestError("retrieval_cases and answer_cases must be lists")
    if not retrieval_cases and not answer_cases:
        raise ManifestError("manifest must contain at least one case")
    if len(retrieval_cases) + len(answer_cases) > MAX_CASES:
        raise ManifestError(f"manifest contains more than {MAX_CASES} cases")
    corpus = _corpus_metadata(value.get("corpus"))
    thresholds = value.get("thresholds", {})
    if not isinstance(thresholds, dict):
        raise ManifestError("thresholds must be an object")
    defaults = {
        "min_recall_at_k": 0.0,
        "min_mrr": 0.0,
        "min_retrieval_pass_rate": 0.0,
        "min_answer_pass_rate": 0.0,
        "min_citation_validity": 0.0,
        "max_retrieval_fallback_rate": 1.0,
        "max_provider_fallback_rate": 1.0,
        "max_latency_ms": 60_000,
    }
    checked_thresholds: dict[str, float | int] = {}
    for name, default in defaults.items():
        threshold = thresholds.get(name, default)
        if name == "max_latency_ms":
            if not isinstance(threshold, int) or isinstance(threshold, bool) or threshold <= 0:
                raise ManifestError("thresholds.max_latency_ms must be a positive integer")
            checked_thresholds[name] = min(threshold, int(MAX_REQUEST_SECONDS * 1000))
        else:
            if (
                not isinstance(threshold, (int, float))
                or isinstance(threshold, bool)
                or not 0 <= threshold <= 1
            ):
                raise ManifestError(f"thresholds.{name} must be between 0 and 1")
            checked_thresholds[name] = float(threshold)
    return {
        "version": 1,
        "corpus": corpus,
        "thresholds": checked_thresholds,
        "retrieval_cases": [
            _case(item, f"retrieval_cases[{index}]") for index, item in enumerate(retrieval_cases)
        ],
        "answer_cases": [
            _case(item, f"answer_cases[{index}]", answer=True)
            for index, item in enumerate(answer_cases)
        ],
    }


def load_manifest(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise ManifestError("manifest is not a regular file")
    metadata = path.stat()
    if metadata.st_size > MAX_MANIFEST_BYTES:
        raise ManifestError(f"manifest exceeds the {MAX_MANIFEST_BYTES} byte limit")
    try:
        raw = path.read_bytes()
        value = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ManifestError("manifest is not valid UTF-8 JSON") from error
    manifest = validate_manifest(value)
    manifest["manifest_digest"] = f"sha256:{hashlib.sha256(raw).hexdigest()}"
    return manifest


def _request_payload(case: Mapping[str, Any]) -> dict[str, Any]:
    payload = {
        "query": case["query"],
        "project": case["project"],
        "source": case["source"],
    }
    if not case["answer"]:
        payload["limit"] = case["top_k"]
    return payload


def _post(
    client: httpx.Client, path: str, case: Mapping[str, Any], *, timeout_seconds: float
) -> tuple[dict[str, Any] | list[Any] | None, int | None, int, dict[str, str]]:
    started = time.perf_counter()
    try:
        with client.stream(
            "POST", path, json=_request_payload(case), timeout=timeout_seconds
        ) as response:
            status = response.status_code
            headers = {key.lower(): value for key, value in response.headers.items()}
            if status >= 400:
                return None, status, round((time.perf_counter() - started) * 1000), headers
            body = bytearray()
            for chunk in response.iter_bytes():
                body.extend(chunk)
                if len(body) > MAX_RESPONSE_BYTES:
                    return None, status, round((time.perf_counter() - started) * 1000), headers
    except httpx.HTTPError:
        return None, None, round((time.perf_counter() - started) * 1000), {}
    latency_ms = round((time.perf_counter() - started) * 1000)
    try:
        payload = json.loads(bytes(body))
    except (TypeError, UnicodeError, json.JSONDecodeError):
        return None, status, latency_ms, headers
    if not isinstance(payload, (dict, list)):
        return None, status, latency_ms, headers
    return payload, status, latency_ms, headers


def _evidence_ids(payload: Any) -> list[str]:
    if not isinstance(payload, (dict, list)):
        return []
    rows = payload if isinstance(payload, list) else payload.get("evidence", [])
    if not isinstance(rows, list):
        return []
    return [
        str(row["source_id"])
        for row in rows
        if isinstance(row, dict) and isinstance(row.get("source_id"), str)
    ]


def _reciprocal_rank(returned: Sequence[str], expected: Sequence[str]) -> float:
    expected_set = set(expected)
    for index, source_id in enumerate(returned):
        if source_id in expected_set:
            return 1.0 / (index + 1)
    return 0.0


def _citation_validity(answer: str, evidence_count: int) -> bool:
    citations = _CITATION.findall(answer)
    return bool(citations) and all(1 <= int(citation) <= evidence_count for citation in citations)


def evaluate_manifest(
    manifest: Mapping[str, Any],
    client: httpx.Client,
    *,
    require_synthesis: bool = False,
    request_seconds: float = DEFAULT_REQUEST_SECONDS,
    total_seconds: float = DEFAULT_TOTAL_SECONDS,
) -> dict[str, Any]:
    started = time.perf_counter()
    retrieval_reports: list[dict[str, Any]] = []
    answer_reports: list[dict[str, Any]] = []
    for case in [*manifest["retrieval_cases"], *manifest["answer_cases"]]:
        remaining = total_seconds - (time.perf_counter() - started)
        if remaining <= 0:
            report = {"name": case["name"], "passed": False, "error": "total_timeout"}
            (answer_reports if case["answer"] else retrieval_reports).append(report)
            continue
        endpoint = "/v1/answer" if case["answer"] else "/v1/search"
        payload, status, latency_ms, headers = _post(
            client, endpoint, case, timeout_seconds=min(request_seconds, remaining)
        )
        returned = _evidence_ids(payload)
        expected = case["expected_source_ids"]
        forbidden = case["forbidden_source_ids"]
        returned_set = set(returned)
        missing = [source_id for source_id in expected if source_id not in returned_set]
        leaked = [source_id for source_id in forbidden if source_id in returned_set]
        if case["answer"]:
            response = payload if isinstance(payload, dict) else {}
            answer = response.get("answer") if isinstance(response.get("answer"), str) else ""
            answer_evidence = response.get("evidence", [])
            source_scope_valid = case["source"] is None or (
                isinstance(answer_evidence, list)
                and all(
                    isinstance(item, dict) and item.get("source") == case["source"]
                    for item in answer_evidence
                )
            )
            citations_valid = _citation_validity(answer, len(returned))
            forbidden_citations_absent = all(
                f"[{index + 1}]" not in answer
                for index, source_id in enumerate(returned)
                if source_id in forbidden
            )
            required_terms = case["required_answer_terms"]
            answer_lower = answer.casefold()
            answer_terms_missing = sum(
                term.casefold() not in answer_lower for term in required_terms
            )
            answer_terms_valid = answer_terms_missing == 0
            synthesis_used = response.get("mode") == "synthesized"
            warnings = response.get("warnings", [])
            fallback_provider_unavailable = isinstance(warnings, list) and any(
                isinstance(warning, str) and "unavailable" in warning.lower()
                for warning in warnings
            )
            remaining = total_seconds - (time.perf_counter() - started)
            if remaining > 0:
                second_payload, second_status, second_latency, _ = _post(
                    client, endpoint, case, timeout_seconds=min(request_seconds, remaining)
                )
            else:
                second_payload, second_status, second_latency = None, None, 0
            cache_hit = isinstance(second_payload, dict) and second_payload.get("cached") is True
            passed = (
                status is not None
                and status < 400
                and not missing
                and not leaked
                and source_scope_valid
                and citations_valid
                and forbidden_citations_absent
                and answer_terms_valid
                and not fallback_provider_unavailable
                and (not require_synthesis or synthesis_used)
            )
            answer_reports.append(
                {
                    "name": case["name"],
                    "passed": passed,
                    "latency_ms": latency_ms,
                    "repeat_latency_ms": second_latency,
                    "returned_source_ids": returned,
                    "missing_source_ids": missing,
                    "leaked_source_ids": leaked,
                    "source_scope_valid": source_scope_valid,
                    "citations_valid": citations_valid,
                    "answer_terms_checked": len(required_terms),
                    "answer_terms_missing": answer_terms_missing,
                    "answer_terms_valid": answer_terms_valid,
                    "forbidden_citations_absent": forbidden_citations_absent,
                    "synthesis_used": synthesis_used,
                    "fallback_provider_unavailable": fallback_provider_unavailable,
                    "retrieval_degraded": response.get("retrieval_degraded") is True,
                    "cache_hit": cache_hit,
                    "cache_checked": second_status is not None,
                    "error_status": status if status is None or status >= 400 else None,
                }
            )
        else:
            source_scope_valid = case["source"] is None or all(
                isinstance(item, dict) and item.get("source") == case["source"]
                for item in (payload if isinstance(payload, list) else [])
            )
            retrieval_reports.append(
                {
                    "name": case["name"],
                    "passed": status is not None
                    and status < 400
                    and not missing
                    and not leaked
                    and source_scope_valid,
                    "latency_ms": latency_ms,
                    "returned_source_ids": returned,
                    "missing_source_ids": missing,
                    "leaked_source_ids": leaked,
                    "retrieval_mode": headers.get("x-cortana-retrieval-mode"),
                    "retrieval_degraded": headers.get("x-cortana-retrieval-degraded") == "true",
                    "error_status": status if status is None or status >= 400 else None,
                }
            )

    retrieval_expected = sum(
        len(case["expected_source_ids"]) for case in manifest["retrieval_cases"]
    )
    retrieval_found = sum(
        len(case["expected_source_ids"]) - len(report["missing_source_ids"])
        for case, report in zip(manifest["retrieval_cases"], retrieval_reports, strict=False)
        if "missing_source_ids" in report
    )
    retrieval_pass_rate = _ratio(
        sum(report.get("passed") is True for report in retrieval_reports), len(retrieval_reports)
    )
    answer_pass_rate = _ratio(
        sum(report.get("passed") is True for report in answer_reports), len(answer_reports)
    )
    citation_validity = _ratio(
        sum(report.get("citations_valid") is True for report in answer_reports), len(answer_reports)
    )
    retrieval_fallback_rate = _ratio(
        sum(report.get("retrieval_degraded") is True for report in retrieval_reports),
        len(retrieval_reports),
    )
    provider_fallback_rate = _ratio(
        sum(report.get("fallback_provider_unavailable") is True for report in answer_reports),
        len(answer_reports),
    )
    mrr_values = [
        _reciprocal_rank(report.get("returned_source_ids", []), case["expected_source_ids"])
        for case, report in zip(manifest["retrieval_cases"], retrieval_reports, strict=False)
        if case["expected_source_ids"]
    ]
    metrics = {
        "recall_at_k": retrieval_found / retrieval_expected if retrieval_expected else 1.0,
        "mrr": sum(mrr_values) / len(mrr_values) if mrr_values else 1.0,
        "retrieval_pass_rate": retrieval_pass_rate,
        "answer_pass_rate": answer_pass_rate,
        "citation_validity": citation_validity,
        "retrieval_fallback_rate": retrieval_fallback_rate,
        "provider_fallback_rate": provider_fallback_rate,
        "max_latency_ms": max(
            [report.get("latency_ms", 0) for report in [*retrieval_reports, *answer_reports]],
            default=0,
        ),
        "cache_hit_rate": _ratio(
            sum(report.get("cache_hit") is True for report in answer_reports), len(answer_reports)
        ),
    }
    thresholds = manifest["thresholds"]
    passed = (
        metrics["recall_at_k"] >= thresholds["min_recall_at_k"]
        and metrics["mrr"] >= thresholds["min_mrr"]
        and metrics["retrieval_pass_rate"] >= thresholds["min_retrieval_pass_rate"]
        and metrics["answer_pass_rate"] >= thresholds["min_answer_pass_rate"]
        and metrics["citation_validity"] >= thresholds["min_citation_validity"]
        and metrics["retrieval_fallback_rate"] <= thresholds["max_retrieval_fallback_rate"]
        and metrics["provider_fallback_rate"] <= thresholds["max_provider_fallback_rate"]
        and metrics["max_latency_ms"] <= thresholds["max_latency_ms"]
    )
    provenance: dict[str, Any] = {}
    manifest_digest = manifest.get("manifest_digest")
    if isinstance(manifest_digest, str):
        provenance["manifest_digest"] = manifest_digest
    corpus = manifest.get("corpus")
    if isinstance(corpus, dict):
        # Reports intentionally carry identifiers and digests only.  Approval
        # timestamps, reviewer labels, and all case content stay local.
        provenance["corpus"] = {
            key: corpus[key] for key in ("id", "revision", "digest") if key in corpus
        }
    return {
        "evaluation": "cortana-live-index-v1",
        "passed": passed,
        "read_only": True,
        "provenance": provenance,
        "cache_invalidation_checked": False,
        "metrics": metrics,
        "thresholds": thresholds,
        "retrieval_cases": retrieval_reports,
        "answer_cases": answer_reports,
    }


def _ratio(numerator: int, denominator: int) -> float:
    return numerator / denominator if denominator else 1.0


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path, help="approved query/expected-source manifest")
    parser.add_argument("--base-url", default="http://127.0.0.1:7331")
    parser.add_argument("--token-env", help="environment variable containing a scoped bearer token")
    parser.add_argument("--request-timeout-seconds", type=float, default=DEFAULT_REQUEST_SECONDS)
    parser.add_argument("--total-timeout-seconds", type=float, default=DEFAULT_TOTAL_SECONDS)
    parser.add_argument("--require-synthesis", action="store_true")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    if not 0 < arguments.request_timeout_seconds <= MAX_REQUEST_SECONDS:
        raise SystemExit(f"--request-timeout-seconds must be between 0 and {MAX_REQUEST_SECONDS}")
    if not 0 < arguments.total_timeout_seconds <= MAX_TOTAL_SECONDS:
        raise SystemExit(f"--total-timeout-seconds must be between 0 and {MAX_TOTAL_SECONDS}")
    parsed = urlparse(arguments.base_url)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise SystemExit("--base-url must be an absolute HTTP(S) URL")
    if parsed.username or parsed.password:
        raise SystemExit("--base-url must not contain embedded credentials")
    hostname = (parsed.hostname or "").lower()
    if hostname not in {"127.0.0.1", "localhost", "::1"} and parsed.scheme != "https":
        raise SystemExit("non-loopback --base-url must use HTTPS")
    try:
        manifest = load_manifest(arguments.manifest)
    except (OSError, ManifestError) as error:
        raise SystemExit(f"invalid live evaluation manifest: {error}") from error
    headers: dict[str, str] = {}
    if arguments.token_env:
        token = os.environ.get(arguments.token_env)
        if not token:
            raise SystemExit(f"{arguments.token_env} is not set")
        headers["Authorization"] = f"Bearer {token}"
    with httpx.Client(
        base_url=arguments.base_url.rstrip("/"),
        headers=headers,
        timeout=httpx.Timeout(arguments.request_timeout_seconds),
        follow_redirects=False,
    ) as client:
        report = evaluate_manifest(
            manifest,
            client,
            require_synthesis=arguments.require_synthesis,
            request_seconds=arguments.request_timeout_seconds,
            total_seconds=arguments.total_timeout_seconds,
        )
    json.dump(report, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
