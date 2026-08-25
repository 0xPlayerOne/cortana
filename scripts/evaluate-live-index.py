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
MAX_GOVERNANCE_LIST = 64
MAX_REVIEWERS = 16
MAX_COVERAGE_ENTRIES = 64
MAX_RETENTION_DAYS = 3650
MAX_MEMORY_MB = 64 * 1024
GOVERNANCE_VERSION = "cortana.approved-corpus.v1"
CASE_MODES = {"retrieval-only", "extractive-answer", "provider-synthesis"}
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
        if any(character in item for character in "\r\n\x00"):
            raise ManifestError(f"{label} contains a control character")
        if item.startswith(("/", "\\")) or re.fullmatch(r"[A-Za-z]:[\\/].*", item):
            raise ManifestError(f"{label} must not contain filesystem paths")
        result.append(item)
    return result


def _bounded_ids(value: Any, label: str) -> list[str]:
    """Validate a bounded list of non-secret scope identifiers."""
    return _source_ids(value, label)


def _governance_ids(value: Any, label: str, *, required: bool = False) -> list[str]:
    """Validate opaque operator identifiers without accepting filesystem paths."""
    if value is None:
        if required:
            raise ManifestError(f"{label} must contain at least one ID")
        return []
    if not isinstance(value, list) or len(value) > MAX_GOVERNANCE_LIST:
        raise ManifestError(f"{label} must be a list of at most {MAX_GOVERNANCE_LIST} IDs")
    result: list[str] = []
    seen: set[str] = set()
    for item in value:
        if not isinstance(item, str) or not item.strip():
            raise ManifestError(f"{label} contains an invalid ID")
        if len(item.encode("utf-8")) > MAX_ID_BYTES:
            raise ManifestError(f"{label} contains an oversized ID")
        if any(character in item for character in "\r\n\x00"):
            raise ManifestError(f"{label} contains a control character")
        if item.startswith(("/", "\\")) or re.fullmatch(r"[A-Za-z]:[\\/].*", item):
            raise ManifestError(f"{label} must not contain filesystem paths")
        if item not in seen:
            seen.add(item)
            result.append(item)
    if required and not result:
        raise ManifestError(f"{label} must contain at least one ID")
    return result


def _governance_id(value: Any, label: str) -> str:
    """Validate one opaque identifier used by a coverage entry."""
    values = _governance_ids([value], label, required=True)
    return values[0]


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


def _governance(value: Any) -> dict[str, Any]:
    """Validate the operator-controlled privacy and evaluation contract."""
    if not isinstance(value, dict):
        raise ManifestError("governance must be an object")
    if value.get("contract_version") != GOVERNANCE_VERSION:
        raise ManifestError(f"governance.contract_version must be {GOVERNANCE_VERSION}")
    for name in (
        "operator_controlled",
        "raw_data_external",
        "credentials_external",
        "private_paths_external",
    ):
        if value.get(name) is not True:
            raise ManifestError(f"governance.{name} must be true")

    scope = value.get("scope")
    if not isinstance(scope, dict):
        raise ManifestError("governance.scope must be an object")
    workspaces = _governance_ids(
        scope.get("workspaces"), "governance.scope.workspaces", required=True
    )
    sources = _governance_ids(scope.get("sources"), "governance.scope.sources", required=True)
    forbidden_sources = _governance_ids(
        scope.get("forbidden_sources"), "governance.scope.forbidden_sources"
    )
    if set(workspaces) & set(forbidden_sources):
        raise ManifestError("governance scope cannot forbid a workspace as a source")
    memory = scope.get("memory", "excluded")
    if memory not in {"excluded", "included"}:
        raise ManifestError("governance.scope.memory must be `excluded` or `included`")

    storage = value.get("storage")
    if not isinstance(storage, dict):
        raise ManifestError("governance.storage must be an object")
    storage_mode = storage.get("mode")
    if storage_mode not in {"local", "encrypted-local"}:
        raise ManifestError("governance.storage.mode must be `local` or `encrypted-local`")
    if storage.get("credentials_external") is not True:
        raise ManifestError("governance.storage.credentials_external must be true")

    reviewer_access = value.get("reviewer_access")
    if not isinstance(reviewer_access, dict):
        raise ManifestError("governance.reviewer_access must be an object")
    access_mode = reviewer_access.get("mode")
    if access_mode not in {"local-only", "encrypted-reviewer-store"}:
        raise ManifestError(
            "governance.reviewer_access.mode must be `local-only` or `encrypted-reviewer-store`"
        )
    reviewers = _governance_ids(
        reviewer_access.get("reviewers"), "governance.reviewer_access.reviewers", required=True
    )
    if reviewer_access.get("approval_required") is not True:
        raise ManifestError("governance.reviewer_access.approval_required must be true")
    if len(reviewers) > MAX_REVIEWERS:
        raise ManifestError(f"governance.reviewer_access.reviewers exceeds {MAX_REVIEWERS}")

    lifecycle = value.get("lifecycle")
    if not isinstance(lifecycle, dict):
        raise ManifestError("governance.lifecycle must be an object")
    retention_days = lifecycle.get("retention_days")
    if (
        not isinstance(retention_days, int)
        or isinstance(retention_days, bool)
        or not 1 <= retention_days <= MAX_RETENTION_DAYS
    ):
        raise ManifestError(
            f"governance.lifecycle.retention_days must be between 1 and {MAX_RETENTION_DAYS}"
        )
    allowed_lifecycle = {
        "deletion": {"operator-confirmed", "reviewer-confirmed"},
        "redaction": {"operator-controlled", "reviewer-controlled"},
        "incident": {"stop-revoke-notify", "stop-revoke"},
    }
    for name, allowed in allowed_lifecycle.items():
        if lifecycle.get(name) not in allowed:
            choices = " or ".join(f"`{item}`" for item in sorted(allowed))
            raise ManifestError(f"governance.lifecycle.{name} must be {choices}")

    bounds = value.get("resource_bounds")
    if not isinstance(bounds, dict):
        raise ManifestError("governance.resource_bounds must be an object")
    max_request_seconds = bounds.get("max_request_seconds")
    max_total_seconds = bounds.get("max_total_seconds")
    max_response_bytes = bounds.get("max_response_bytes")
    max_memory_mb = bounds.get("max_memory_mb")
    if (
        not isinstance(max_request_seconds, (int, float))
        or isinstance(max_request_seconds, bool)
        or not 0 < max_request_seconds <= MAX_REQUEST_SECONDS
    ):
        raise ManifestError(
            f"governance.resource_bounds.max_request_seconds must be between 0 and {MAX_REQUEST_SECONDS}"
        )
    if (
        not isinstance(max_total_seconds, (int, float))
        or isinstance(max_total_seconds, bool)
        or not max_request_seconds <= max_total_seconds <= MAX_TOTAL_SECONDS
    ):
        raise ManifestError(
            "governance.resource_bounds.max_total_seconds must cover the request bound"
        )
    if (
        not isinstance(max_response_bytes, int)
        or isinstance(max_response_bytes, bool)
        or not 1 <= max_response_bytes <= MAX_RESPONSE_BYTES
    ):
        raise ManifestError(
            f"governance.resource_bounds.max_response_bytes must be between 1 and {MAX_RESPONSE_BYTES}"
        )
    if (
        not isinstance(max_memory_mb, int)
        or isinstance(max_memory_mb, bool)
        or not 1 <= max_memory_mb <= MAX_MEMORY_MB
    ):
        raise ManifestError("governance.resource_bounds.max_memory_mb is outside the safety bound")
    max_cases = bounds.get("max_cases", MAX_CASES)
    if (
        not isinstance(max_cases, int)
        or isinstance(max_cases, bool)
        or not 1 <= max_cases <= MAX_CASES
    ):
        raise ManifestError(
            f"governance.resource_bounds.max_cases must be between 1 and {MAX_CASES}"
        )

    coverage = value.get("coverage")
    if not isinstance(coverage, list) or not coverage or len(coverage) > MAX_COVERAGE_ENTRIES:
        raise ManifestError("governance.coverage must be a non-empty bounded list")
    checked_coverage: list[dict[str, Any]] = []
    for index, item in enumerate(coverage):
        label = f"governance.coverage[{index}]"
        if not isinstance(item, dict):
            raise ManifestError(f"{label} must be an object")
        coverage_source = _governance_id(item.get("source"), f"{label}.source")
        if coverage_source not in sources:
            raise ManifestError(f"{label}.source must be one configured source")
        coverage_workspace = _governance_id(item.get("workspace"), f"{label}.workspace")
        if coverage_workspace not in workspaces:
            raise ManifestError(f"{label}.workspace must be one configured workspace")
        minimum_cases = item.get("minimum_cases", 1)
        if (
            not isinstance(minimum_cases, int)
            or isinstance(minimum_cases, bool)
            or not 0 <= minimum_cases <= MAX_CASES
        ):
            raise ManifestError(f"{label}.minimum_cases must be between 0 and {MAX_CASES}")
        checked_coverage.append(
            {
                "workspace": coverage_workspace,
                "source": coverage_source,
                "minimum_cases": minimum_cases,
            }
        )

    provider_synthesis_enabled = value.get("provider_synthesis_enabled", False)
    if not isinstance(provider_synthesis_enabled, bool):
        raise ManifestError("governance.provider_synthesis_enabled must be a boolean")
    return {
        "contract_version": GOVERNANCE_VERSION,
        "operator_controlled": True,
        "raw_data_external": True,
        "credentials_external": True,
        "private_paths_external": True,
        "scope": {
            "workspaces": workspaces,
            "sources": sources,
            "forbidden_sources": forbidden_sources,
            "memory": memory,
        },
        "storage": {"mode": storage_mode, "credentials_external": True},
        "reviewer_access": {
            "mode": access_mode,
            "reviewers": reviewers,
            "approval_required": True,
        },
        "lifecycle": {
            "retention_days": retention_days,
            "deletion": lifecycle["deletion"],
            "redaction": lifecycle["redaction"],
            "incident": lifecycle["incident"],
        },
        "resource_bounds": {
            "max_request_seconds": float(max_request_seconds),
            "max_total_seconds": float(max_total_seconds),
            "max_response_bytes": max_response_bytes,
            "max_memory_mb": max_memory_mb,
            "max_cases": max_cases,
        },
        "coverage": checked_coverage,
        "provider_synthesis_enabled": provider_synthesis_enabled,
    }


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


def _case(
    value: Any,
    label: str,
    *,
    answer: bool = False,
    context: bool = False,
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ManifestError(f"{label} must be an object")
    name = _bounded_text(value.get("name"), f"{label}.name", required=True)
    case_id = _bounded_text(value.get("id", name), f"{label}.id", required=True)
    query = _bounded_text(value.get("query"), f"{label}.query", required=True)
    project = _bounded_text(value.get("project"), f"{label}.project")
    workspace = _bounded_text(value.get("workspace", project), f"{label}.workspace")
    if project is not None and workspace is not None and project != workspace:
        raise ManifestError(f"{label}.project and {label}.workspace must match")
    source = _bounded_text(value.get("source"), f"{label}.source")
    top_k = value.get("top_k", 10)
    if not isinstance(top_k, int) or isinstance(top_k, bool) or not 1 <= top_k <= 50:
        raise ManifestError(f"{label}.top_k must be between 1 and 50")
    max_tokens = value.get("max_tokens", 8_000)
    if context and (
        not isinstance(max_tokens, int)
        or isinstance(max_tokens, bool)
        or not 256 <= max_tokens <= 64_000
    ):
        raise ManifestError(f"{label}.max_tokens must be between 256 and 64000")
    mode = value.get("mode")
    default_mode = "extractive-answer" if answer else "retrieval-only"
    if mode is None:
        mode = default_mode
    if (
        mode not in CASE_MODES
        or (answer is False and mode != "retrieval-only")
        or (answer is True and mode == "retrieval-only")
    ):
        raise ManifestError(f"{label}.mode must be a valid mode for this case list")
    expected_value = value.get("expected_evidence_ids", value.get("expected_source_ids"))
    forbidden_value = value.get("forbidden_evidence_ids", value.get("forbidden_source_ids"))
    expected_source_ids = _source_ids(expected_value, f"{label}.expected_evidence_ids")
    if not expected_source_ids:
        raise ManifestError(f"{label}.expected_evidence_ids must contain at least one ID")
    criteria = value.get("answer_criteria", {})
    if not isinstance(criteria, dict):
        raise ManifestError(f"{label}.answer_criteria must be an object")
    required_terms_value = criteria.get("required_terms", value.get("required_answer_terms"))
    min_citations = criteria.get("min_citations", 1 if answer else 0)
    allow_abstain = criteria.get("allow_abstain", False)
    if (
        not isinstance(min_citations, int)
        or isinstance(min_citations, bool)
        or not 0 <= min_citations <= 32
    ):
        raise ManifestError(f"{label}.answer_criteria.min_citations must be between 0 and 32")
    if not isinstance(allow_abstain, bool):
        raise ManifestError(f"{label}.answer_criteria.allow_abstain must be a boolean")
    return {
        "name": name,
        "id": case_id,
        "query": query,
        "project": workspace,
        "workspace": workspace,
        "source": source,
        "top_k": top_k,
        "max_tokens": max_tokens,
        "expected_source_ids": expected_source_ids,
        "forbidden_source_ids": _source_ids(forbidden_value, f"{label}.forbidden_evidence_ids"),
        "forbidden_projects": _bounded_ids(
            value.get("forbidden_projects"), f"{label}.forbidden_projects"
        ),
        "forbidden_sources": _bounded_ids(
            value.get("forbidden_sources"), f"{label}.forbidden_sources"
        ),
        "required_answer_terms": _answer_terms(
            required_terms_value, f"{label}.answer_criteria.required_terms"
        )
        if answer
        else [],
        "min_citations": min_citations if answer else 0,
        "allow_abstain": allow_abstain if answer else False,
        "mode": mode,
        "answer": answer,
    }


def validate_manifest(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict) or value.get("version") != 1:
        raise ManifestError("manifest version must be 1")
    retrieval_cases = value.get("retrieval_cases", [])
    context_cases = value.get("context_cases", [])
    answer_cases = value.get("answer_cases", [])
    if (
        not isinstance(retrieval_cases, list)
        or not isinstance(context_cases, list)
        or not isinstance(answer_cases, list)
    ):
        raise ManifestError("retrieval_cases, context_cases, and answer_cases must be lists")
    if not retrieval_cases and not context_cases and not answer_cases:
        raise ManifestError("manifest must contain at least one case")
    if len(retrieval_cases) + len(context_cases) + len(answer_cases) > MAX_CASES:
        raise ManifestError(f"manifest contains more than {MAX_CASES} cases")
    corpus = _corpus_metadata(value.get("corpus"))
    governance = _governance(value.get("governance"))
    if corpus is not None and corpus["storage"] != governance["storage"]["mode"]:
        raise ManifestError("corpus.storage must match governance.storage.mode")
    checked_retrieval_cases = [
        _case(item, f"retrieval_cases[{index}]") for index, item in enumerate(retrieval_cases)
    ]
    checked_context_cases = [
        _case(item, f"context_cases[{index}]", context=True)
        for index, item in enumerate(context_cases)
    ]
    checked_answer_cases = [
        _case(item, f"answer_cases[{index}]", answer=True)
        for index, item in enumerate(answer_cases)
    ]
    cases = [*checked_retrieval_cases, *checked_context_cases, *checked_answer_cases]
    if len(cases) > governance["resource_bounds"]["max_cases"]:
        raise ManifestError("manifest exceeds governance.resource_bounds.max_cases")
    case_ids = [case["id"] for case in cases]
    if len(set(case_ids)) != len(case_ids):
        raise ManifestError("case IDs must be unique")
    allowed_workspaces = set(governance["scope"]["workspaces"])
    allowed_sources = set(governance["scope"]["sources"])
    forbidden_sources = set(governance["scope"]["forbidden_sources"])
    for case in cases:
        if case["workspace"] is not None and case["workspace"] not in allowed_workspaces:
            raise ManifestError(f"case {case['id']} uses a workspace outside governance scope")
        if case["source"] is not None and case["source"] not in allowed_sources:
            raise ManifestError(f"case {case['id']} uses a source outside governance scope")
        if case["source"] in forbidden_sources:
            raise ManifestError(f"case {case['id']} uses a forbidden source")
        if case["mode"] == "provider-synthesis" and not governance["provider_synthesis_enabled"]:
            raise ManifestError("provider-synthesis cases require explicit governance opt-in")
    for coverage in governance["coverage"]:
        covered = sum(
            case["workspace"] == coverage["workspace"] and case["source"] == coverage["source"]
            for case in cases
        )
        if covered < coverage["minimum_cases"]:
            raise ManifestError(
                f"governance coverage is incomplete for {coverage['workspace']}/{coverage['source']}"
            )
    thresholds = value.get("thresholds", {})
    if not isinstance(thresholds, dict):
        raise ManifestError("thresholds must be an object")
    defaults = {
        "min_recall_at_k": 0.0,
        "min_mrr": 0.0,
        "min_retrieval_pass_rate": 0.0,
        "min_context_pass_rate": 0.0,
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
        "governance": governance,
        "thresholds": checked_thresholds,
        "retrieval_cases": checked_retrieval_cases,
        "context_cases": checked_context_cases,
        "answer_cases": checked_answer_cases,
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


def _request_payload(case: Mapping[str, Any], endpoint: str) -> dict[str, Any]:
    payload = {
        "query": case["query"],
        "project": case["project"],
        "source": case["source"],
    }
    if endpoint in {"/v1/search", "/v1/context"}:
        payload["limit"] = case["top_k"]
    if endpoint == "/v1/context":
        payload["max_tokens"] = case["max_tokens"]
    return payload


def _post(
    client: httpx.Client,
    path: str,
    case: Mapping[str, Any],
    *,
    timeout_seconds: float,
    max_response_bytes: int = MAX_RESPONSE_BYTES,
) -> tuple[dict[str, Any] | list[Any] | None, int | None, int, dict[str, str]]:
    started = time.perf_counter()
    try:
        with client.stream(
            "POST", path, json=_request_payload(case, path), timeout=timeout_seconds
        ) as response:
            status = response.status_code
            headers = {key.lower(): value for key, value in response.headers.items()}
            if status >= 400:
                return None, status, round((time.perf_counter() - started) * 1000), headers
            body = bytearray()
            for chunk in response.iter_bytes():
                body.extend(chunk)
                if len(body) > max_response_bytes:
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


def _evidence_rows(payload: Any) -> list[dict[str, Any]]:
    if not isinstance(payload, (dict, list)):
        return []
    rows = payload if isinstance(payload, list) else payload.get("evidence", [])
    if not isinstance(rows, list):
        return []
    return [row for row in rows if isinstance(row, dict)]


def _percentile(values: Sequence[int], percentile: float) -> int:
    """Return a deterministic nearest-rank percentile for bounded samples."""
    if not values:
        return 0
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, int((percentile * len(ordered)) + 0.999999) - 1))
    return ordered[index]


def _source_metrics(rows: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    sources = [str(row["source"]) for row in rows if isinstance(row.get("source"), str)]
    unique = len(set(sources))
    returned = len(rows)
    counts: dict[str, int] = {}
    for source in sources:
        counts[source] = counts.get(source, 0) + 1
    duplicate_rows = sum(max(0, count - 1) for count in counts.values())
    return {
        "unique_sources": unique,
        "source_diversity": unique / returned if returned else 1.0,
        "duplicate_source_rows": duplicate_rows,
        "duplicate_source_crowding": duplicate_rows / returned if returned else 0.0,
    }


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
    bounds = manifest["governance"]["resource_bounds"]
    request_seconds = min(request_seconds, bounds["max_request_seconds"])
    total_seconds = min(total_seconds, bounds["max_total_seconds"])
    max_response_bytes = bounds["max_response_bytes"]
    started = time.perf_counter()
    retrieval_reports: list[dict[str, Any]] = []
    context_reports: list[dict[str, Any]] = []
    answer_reports: list[dict[str, Any]] = []
    latency_samples: list[int] = []
    cases = [(case, "/v1/search", retrieval_reports) for case in manifest["retrieval_cases"]]
    cases.extend((case, "/v1/context", context_reports) for case in manifest["context_cases"])
    cases.extend((case, "/v1/answer", answer_reports) for case in manifest["answer_cases"])
    for case, endpoint, report_bucket in cases:
        remaining = total_seconds - (time.perf_counter() - started)
        if remaining <= 0:
            report = {"name": case["name"], "passed": False, "error": "total_timeout"}
            report_bucket.append(report)
            continue
        payload, status, latency_ms, headers = _post(
            client,
            endpoint,
            case,
            timeout_seconds=min(request_seconds, remaining),
            max_response_bytes=max_response_bytes,
        )
        latency_samples.append(latency_ms)
        returned = _evidence_ids(payload)
        rows = _evidence_rows(payload)
        expected = case["expected_source_ids"]
        forbidden = case["forbidden_source_ids"]
        returned_set = set(returned)
        missing = [source_id for source_id in expected if source_id not in returned_set]
        leaked = [source_id for source_id in forbidden if source_id in returned_set]
        forbidden_project_leaks = sorted(
            {
                str(row["project"])
                for row in rows
                if isinstance(row.get("project"), str)
                and row["project"] in case["forbidden_projects"]
            }
        )
        forbidden_source_leaks = sorted(
            {
                str(row["source"])
                for row in rows
                if isinstance(row.get("source"), str) and row["source"] in case["forbidden_sources"]
            }
        )
        project_values = [row.get("project") for row in rows if "project" in row]
        project_scope_valid = (
            case["project"] is None
            or not project_values
            or all(value == case["project"] for value in project_values)
        )
        source_values = [row.get("source") for row in rows if "source" in row]
        source_scope_valid = (
            case["source"] is None
            or not source_values
            or all(value == case["source"] for value in source_values)
        )
        if case["answer"]:
            response = payload if isinstance(payload, dict) else {}
            answer = response.get("answer") if isinstance(response.get("answer"), str) else ""
            answer_evidence = response.get("evidence", [])
            answer_rows = (
                [item for item in answer_evidence if isinstance(item, dict)]
                if isinstance(answer_evidence, list)
                else []
            )
            answer_project_values = [
                item.get("project") for item in answer_rows if "project" in item
            ]
            project_scope_valid = (
                case["project"] is None
                or not answer_project_values
                or all(value == case["project"] for value in answer_project_values)
            )
            answer_source_values = [item.get("source") for item in answer_rows if "source" in item]
            source_scope_valid = (
                case["source"] is None
                or not answer_source_values
                or all(value == case["source"] for value in answer_source_values)
            )
            citation_values = _CITATION.findall(answer)
            citations_valid = (
                _citation_validity(answer, len(returned))
                and len(citation_values) >= case["min_citations"]
            ) or (case["allow_abstain"] and not answer)
            citation_indices = {int(citation) - 1 for citation in citation_values}
            forbidden_citations_absent = all(
                index not in citation_indices
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
            answer_mode_valid = response.get("mode") == (
                "synthesized" if case["mode"] == "provider-synthesis" else "extractive"
            )
            warnings = response.get("warnings", [])
            fallback_provider_unavailable = isinstance(warnings, list) and any(
                isinstance(warning, str) and "unavailable" in warning.lower()
                for warning in warnings
            )
            remaining = total_seconds - (time.perf_counter() - started)
            if remaining > 0:
                second_payload, second_status, second_latency, _ = _post(
                    client,
                    endpoint,
                    case,
                    timeout_seconds=min(request_seconds, remaining),
                    max_response_bytes=max_response_bytes,
                )
            else:
                second_payload, second_status, second_latency = None, None, 0
            cache_hit = isinstance(second_payload, dict) and second_payload.get("cached") is True
            passed = (
                status is not None
                and status < 400
                and not missing
                and not leaked
                and not forbidden_project_leaks
                and not forbidden_source_leaks
                and project_scope_valid
                and source_scope_valid
                and citations_valid
                and forbidden_citations_absent
                and answer_terms_valid
                and not fallback_provider_unavailable
                and answer_mode_valid
                and (not require_synthesis or synthesis_used)
            )
            answer_reports.append(
                {
                    "name": case["name"],
                    "case_id": case["id"],
                    "mode": case["mode"],
                    "passed": passed,
                    "latency_ms": latency_ms,
                    "repeat_latency_ms": second_latency,
                    "returned_source_ids": returned,
                    "missing_source_ids": missing,
                    "leaked_source_ids": leaked,
                    "forbidden_project_leaks": forbidden_project_leaks,
                    "forbidden_source_leaks": forbidden_source_leaks,
                    "project_scope_valid": project_scope_valid,
                    "source_scope_valid": source_scope_valid,
                    "citations_valid": citations_valid,
                    "citation_count": len(citation_values),
                    "min_citations": case["min_citations"],
                    "answer_mode_valid": answer_mode_valid,
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
                    **_source_metrics(answer_rows),
                }
            )
        elif endpoint == "/v1/context":
            response = payload if isinstance(payload, dict) else {}
            context_metrics = response.get("metrics", {})
            if not isinstance(context_metrics, dict):
                context_metrics = {}
            required_metric_names = (
                "retrieved",
                "included",
                "omitted",
                "estimated_tokens",
                "max_tokens",
            )
            metrics_valid = all(
                isinstance(context_metrics.get(key), int)
                and not isinstance(context_metrics.get(key), bool)
                and context_metrics[key] >= 0
                for key in required_metric_names
            )
            token_budget_valid = metrics_valid and (
                context_metrics["estimated_tokens"] <= context_metrics["max_tokens"]
            )
            # A repeated read-only context request is the operator-visible
            # digest/content reuse check.  The server may still rebuild the
            # bundle, but unchanged pinned inputs must produce the same digest
            # and context bytes without exposing either in the report.
            remaining = total_seconds - (time.perf_counter() - started)
            if remaining > 0 and status is not None and status < 400:
                second_payload, second_status, second_latency, _ = _post(
                    client,
                    endpoint,
                    case,
                    timeout_seconds=min(request_seconds, remaining),
                    max_response_bytes=max_response_bytes,
                )
                latency_samples.append(second_latency)
            else:
                second_payload, second_status = None, None
            second_response = second_payload if isinstance(second_payload, dict) else {}
            digest = response.get("canonical_digest")
            second_digest = second_response.get("canonical_digest")
            digest_checked = isinstance(digest, str) and isinstance(second_digest, str)
            digest_reused = digest_checked and digest == second_digest
            content_unchanged = digest_reused and response.get("context") == second_response.get(
                "context"
            )
            context_reports.append(
                {
                    "name": case["name"],
                    "passed": (
                        status is not None
                        and status < 400
                        and not missing
                        and not leaked
                        and not forbidden_project_leaks
                        and not forbidden_source_leaks
                        and project_scope_valid
                        and source_scope_valid
                        and token_budget_valid
                    ),
                    "latency_ms": latency_ms,
                    "returned_source_ids": returned,
                    "missing_source_ids": missing,
                    "leaked_source_ids": leaked,
                    "forbidden_project_leaks": forbidden_project_leaks,
                    "forbidden_source_leaks": forbidden_source_leaks,
                    "project_scope_valid": project_scope_valid,
                    "source_scope_valid": source_scope_valid,
                    "retrieval_mode": response.get("retrieval_mode"),
                    "retrieval_degraded": response.get("retrieval_mode") == "lexical-fallback",
                    "metrics": {
                        key: context_metrics[key]
                        for key in (
                            "retrieved",
                            "included",
                            "omitted",
                            "memories_retrieved",
                            "memories_included",
                            "memories_omitted",
                            "estimated_tokens",
                            "max_tokens",
                        )
                        if key in context_metrics
                    },
                    "token_budget_valid": token_budget_valid,
                    "digest_checked": digest_checked and second_status is not None,
                    "digest_reused": digest_reused,
                    "content_unchanged": content_unchanged,
                    "error_status": status if status is None or status >= 400 else None,
                    **_source_metrics(rows),
                }
            )
        else:
            retrieval_reports.append(
                {
                    "name": case["name"],
                    "case_id": case["id"],
                    "mode": case["mode"],
                    "passed": status is not None
                    and status < 400
                    and not missing
                    and not leaked
                    and not forbidden_project_leaks
                    and not forbidden_source_leaks
                    and project_scope_valid
                    and source_scope_valid,
                    "latency_ms": latency_ms,
                    "returned_source_ids": returned,
                    "missing_source_ids": missing,
                    "leaked_source_ids": leaked,
                    "forbidden_project_leaks": forbidden_project_leaks,
                    "forbidden_source_leaks": forbidden_source_leaks,
                    "project_scope_valid": project_scope_valid,
                    "retrieval_mode": headers.get("x-cortana-retrieval-mode"),
                    "retrieval_degraded": headers.get("x-cortana-retrieval-degraded") == "true",
                    "error_status": status if status is None or status >= 400 else None,
                    **_source_metrics(rows),
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
    context_pass_rate = _ratio(
        sum(report.get("passed") is True for report in context_reports), len(context_reports)
    )
    citation_validity = _ratio(
        sum(report.get("citations_valid") is True for report in answer_reports), len(answer_reports)
    )
    retrieval_fallback_rate = _ratio(
        sum(
            report.get("retrieval_degraded") is True
            for report in [*retrieval_reports, *context_reports, *answer_reports]
        ),
        len(retrieval_reports) + len(context_reports) + len(answer_reports),
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
    all_reports = [*retrieval_reports, *context_reports, *answer_reports]
    diversity_values = [
        report["source_diversity"] for report in all_reports if "source_diversity" in report
    ]
    crowding_values = [
        report["duplicate_source_crowding"]
        for report in all_reports
        if "duplicate_source_crowding" in report
    ]
    context_metric_rows = [report.get("metrics", {}) for report in context_reports]
    retrieved_context_rows = sum(
        metric.get("retrieved", 0)
        for metric in context_metric_rows
        if isinstance(metric.get("retrieved", 0), int)
    )
    included_context_rows = sum(
        metric.get("included", 0)
        for metric in context_metric_rows
        if isinstance(metric.get("included", 0), int)
    )
    omitted_context_rows = sum(
        metric.get("omitted", 0)
        for metric in context_metric_rows
        if isinstance(metric.get("omitted", 0), int)
    )
    metrics = {
        "recall_at_k": retrieval_found / retrieval_expected if retrieval_expected else 1.0,
        "mrr": sum(mrr_values) / len(mrr_values) if mrr_values else 1.0,
        "retrieval_pass_rate": retrieval_pass_rate,
        "context_pass_rate": context_pass_rate,
        "answer_pass_rate": answer_pass_rate,
        "citation_validity": citation_validity,
        "retrieval_fallback_rate": retrieval_fallback_rate,
        "provider_fallback_rate": provider_fallback_rate,
        "max_latency_ms": max(latency_samples, default=0),
        "latency_ms_p50": _percentile(latency_samples, 0.50),
        "latency_ms_p95": _percentile(latency_samples, 0.95),
        "latency_ms_p99": _percentile(latency_samples, 0.99),
        "source_diversity": (
            sum(diversity_values) / len(diversity_values) if diversity_values else 1.0
        ),
        "duplicate_source_crowding": (
            sum(crowding_values) / len(crowding_values) if crowding_values else 0.0
        ),
        "forbidden_source_leak_count": sum(
            len(report.get("leaked_source_ids", []))
            + len(report.get("forbidden_project_leaks", []))
            + len(report.get("forbidden_source_leaks", []))
            for report in all_reports
        ),
        "invalid_citation_count": sum(
            report.get("citations_valid") is False for report in answer_reports
        ),
        "token_inclusion_rate": _ratio(included_context_rows, retrieved_context_rows),
        "token_omission_rate": _ratio(omitted_context_rows, retrieved_context_rows),
        "token_budget_compliance_rate": _ratio(
            sum(report.get("token_budget_valid") is True for report in context_reports),
            len(context_reports),
        ),
        "context_digest_reuse_rate": _ratio(
            sum(report.get("digest_reused") is True for report in context_reports),
            sum(report.get("digest_checked") is True for report in context_reports),
        ),
        "context_content_unchanged_rate": _ratio(
            sum(report.get("content_unchanged") is True for report in context_reports),
            sum(report.get("digest_checked") is True for report in context_reports),
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
        and metrics["context_pass_rate"] >= thresholds["min_context_pass_rate"]
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
    governance = manifest.get("governance")
    if isinstance(governance, dict):
        provenance["governance"] = {
            "contract_version": governance["contract_version"],
            "scope_digest": "sha256:"
            + hashlib.sha256(
                json.dumps(governance["scope"], sort_keys=True, separators=(",", ":")).encode()
            ).hexdigest(),
            "provider_synthesis_enabled": governance["provider_synthesis_enabled"],
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
        "context_cases": context_reports,
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
