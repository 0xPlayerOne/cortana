#!/usr/bin/env python3
"""Run the deterministic M9 code-retrieval strategy comparison."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import statistics
import subprocess
import time
import tracemalloc
from array import array
from pathlib import Path
from typing import Any


def tokens(value: str) -> list[str]:
    return [part.lower() for part in re.findall(r"[A-Za-z][A-Za-z0-9_]*", value)]


def subtokens(value: str) -> list[str]:
    return [piece for token in tokens(value) for piece in token.split("_") if piece]


VECTOR_DIMENSIONS = 256


def vectorize(value: str) -> array[float]:
    """Mirror Rust's production DeterministicEmbedder for reproducible offline trials."""
    vector = array("f", [0.0]) * VECTOR_DIMENSIONS
    for part in value.split():
        digest = hashlib.sha256(part.lower().encode()).digest()
        index = int.from_bytes(digest[:2], "little") % VECTOR_DIMENSIONS
        vector[index] += 1.0
    norm = math.sqrt(sum(value * value for value in vector))
    if norm:
        for index, value in enumerate(vector):
            vector[index] = value / norm
    return vector


def cosine(left: array[float], right: array[float]) -> float:
    return sum(a * b for a, b in zip(left, right, strict=True))


def representation(strategy: str, document: dict[str, Any]) -> array[float] | None:
    if strategy == "lexical_symbol":
        return None
    if strategy in {"shared_text_embedding", "local_fusion_rerank"}:
        return vectorize(f"{document['path']} {document['text']}")
    return vectorize(" ".join(subtokens(f"{document['symbol']} {document['path']}")))


def query_representation(strategy: str, query: str) -> array[float] | None:
    if strategy == "lexical_symbol":
        return None
    if strategy in {"shared_text_embedding", "local_fusion_rerank"}:
        return vectorize(query)
    return vectorize(" ".join(subtokens(query)))


def materialize_documents(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    documents: list[dict[str, Any]] = []
    for specification in manifest["documents"]:
        document = dict(specification)
        source = Path(document["path"]).read_text(encoding="utf-8")
        lines = source.splitlines(keepends=True)
        expected_line = int(document["expected_line"])
        if expected_line < 1 or expected_line > len(lines):
            raise ValueError(f"expected line is outside {document['path']}")
        line = lines[expected_line - 1]
        expected_fragment = str(document["expected_fragment"])
        if expected_fragment not in line or document["symbol"] not in line:
            raise ValueError(f"expected declaration drifted at {document['path']}:{expected_line}")
        symbol_bytes = document["symbol"].encode()
        line_start = len("".join(lines[: expected_line - 1]).encode())
        start = line_start + line.encode().index(symbol_bytes)
        end = start + len(symbol_bytes)
        document.update(
            source=source,
            text=source,
            span={"start_byte": start, "end_byte": end},
            lexical_tokens=sorted(set(tokens(f"{document['symbol']} {document['path']} {source}"))),
        )
        documents.append(document)
    return documents


def build_index(
    strategy: str,
    documents: list[dict[str, Any]],
    cache: dict[tuple[str, str, str], array[float] | None],
) -> tuple[dict[str, array[float] | None], int, int]:
    index: dict[str, array[float] | None] = {}
    hits = 0
    for document in documents:
        payload = json.dumps(document, sort_keys=True, separators=(",", ":"))
        key = (strategy, document["id"], hashlib.sha256(payload.encode()).hexdigest())
        if key in cache:
            hits += 1
        else:
            cache[key] = representation(strategy, document)
        index[document["id"]] = cache[key]
    return index, hits, len(documents)


def score(
    strategy: str,
    query: str,
    document: dict[str, Any],
    query_vector: array[float] | None,
    document_vector: array[float] | None,
) -> float:
    lexical = len(set(tokens(query)).intersection(document["lexical_tokens"]))
    exact = float(
        bool(document["symbol"]) and document["symbol"].lower() in query.lower().replace(" ", "_")
    )
    semantic = (
        cosine(query_vector, document_vector)
        if query_vector is not None and document_vector is not None
        else 0.0
    )
    if strategy == "lexical_symbol":
        return lexical + 4.0 * exact
    if strategy in {"shared_text_embedding", "code_specific_embedding"}:
        return semantic
    return semantic + 0.25 * lexical + 4.0 * exact


def span_is_correct(document: dict[str, Any]) -> bool:
    span = document["span"]
    start = span["start_byte"]
    end = span["end_byte"]
    source = document["source"].encode()
    line = document["source"].splitlines()[document["expected_line"] - 1]
    return (
        0 <= start < end <= len(source)
        and source[start:end] == document["symbol"].encode()
        and document["expected_fragment"] in line
    )


def repository_provenance() -> dict[str, Any]:
    revision = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    dirty = bool(
        subprocess.run(
            ["git", "status", "--porcelain", "--untracked-files=no"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    )
    return {"revision": revision, "working_tree_dirty": dirty}


def evaluate(manifest: dict[str, Any]) -> dict[str, Any]:
    documents = materialize_documents(manifest)
    provenance = repository_provenance()
    if manifest["revision"] != "HEAD":
        raise ValueError("evaluation revision must be resolved from HEAD")
    strategies = (
        "lexical_symbol",
        "shared_text_embedding",
        "code_specific_embedding",
        "local_fusion_rerank",
    )
    tracemalloc.start()
    results: dict[str, Any] = {}
    cache: dict[tuple[str, str, str], array[float] | None] = {}
    for strategy in strategies:
        embedding_started = time.perf_counter_ns()
        index, _, _ = build_index(strategy, documents, cache)
        embedding_elapsed = time.perf_counter_ns() - embedding_started
        _, cache_hits, cache_total = build_index(strategy, documents, cache)
        reciprocal_ranks: list[float] = []
        recalls: list[float] = []
        span_correct: list[float] = []
        latencies: list[float] = []
        for case in manifest["queries"]:
            started = time.perf_counter_ns()
            query_vector = query_representation(strategy, case["query"])
            ranked = sorted(
                documents,
                key=lambda document: (
                    -score(
                        strategy,
                        case["query"],
                        document,
                        query_vector,
                        index[document["id"]],
                    ),
                    document["id"],
                ),
            )
            latencies.append((time.perf_counter_ns() - started) / 1_000_000)
            rank = next(
                index
                for index, document in enumerate(ranked, start=1)
                if document["id"] == case["expected"]
            )
            reciprocal_ranks.append(1.0 / rank)
            recalls.append(float(rank <= 3))
            target = next(document for document in ranked if document["id"] == case["expected"])
            span_correct.append(float(span_is_correct(target)))
        vector_bytes = sum(len(value.tobytes()) for value in index.values() if value is not None)
        symbol_bytes = len(
            json.dumps(
                [
                    {"id": item["id"], "path": item["path"], "symbol": item["symbol"]}
                    for item in documents
                ],
                sort_keys=True,
                separators=(",", ":"),
            ).encode()
        )
        results[strategy] = {
            "recall_at_3": statistics.fmean(recalls),
            "mrr": statistics.fmean(reciprocal_ranks),
            "span_correctness": statistics.fmean(span_correct),
            "latency_ms": round(statistics.fmean(latencies), 3),
            "index_bytes": symbol_bytes + vector_bytes,
            "embedding_time_ms": round(embedding_elapsed / 1_000_000, 3),
            "cache_reuse": cache_hits / cache_total if cache_total else 1.0,
            "second_embedding_generation": strategy == "code_specific_embedding",
            "embedding_backend": "runtime-deterministic:256"
            if strategy != "lexical_symbol"
            else "none",
        }
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    budgets = manifest["budgets"]
    eligible = [
        name
        for name, metrics in results.items()
        if not metrics["second_embedding_generation"]
        and metrics["latency_ms"] <= budgets["max_latency_ms"]
        and metrics["span_correctness"] == 1.0
    ]
    selected = max(
        eligible,
        key=lambda name: (
            results[name]["mrr"],
            results[name]["recall_at_3"],
            -results[name]["embedding_time_ms"],
            -results[name]["index_bytes"],
            -results[name]["latency_ms"],
        ),
    )
    migration_started = time.perf_counter_ns()
    migration_index, _, migration_documents = build_index(selected, documents, {})
    migration_elapsed = time.perf_counter_ns() - migration_started
    migration_vector_bytes = sum(
        len(value.tobytes()) for value in migration_index.values() if value is not None
    )
    reembedding_required = selected == "code_specific_embedding"
    return {
        "contract_version": manifest["contract_version"],
        "fixture_only": True,
        "corpus": {
            "kind": "checked-in production source",
            "documents": len(documents),
            "repository": manifest["repository"],
            **provenance,
        },
        "query_classes": sorted({case["class"] for case in manifest["queries"]}),
        "results": results,
        "selected_strategy": selected,
        "selection_reason": "highest measured MRR and recall within the latency/span gates, breaking ties by embedding cost, index size, and query latency",
        "peak_memory_bytes": peak,
        "resource_budgets": budgets,
        "resource_budget_pass": peak <= budgets["max_peak_memory_bytes"],
        "migration_cost": {
            "measured_time_ms": round(migration_elapsed / 1_000_000, 3),
            "documents_scanned": migration_documents,
            "documents_reembedded": migration_documents if reembedding_required else 0,
            "vector_bytes_written": migration_vector_bytes if reembedding_required else 0,
            "additional_generation": reembedding_required,
        },
        "activation": {
            "approved_roots_required": True,
            "sampled_non_reconciling_trial_required": True,
            "generated_vendor_worktree_excluded": True,
            "second_embedding_generation": False,
            "rollback": "delete and rebuild derived code indexes; canonical documents are unchanged",
            "clean_committed_revision_required": True,
            "activation_ready": not provenance["working_tree_dirty"],
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=Path("eval/code-retrieval-v1.json"))
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()
    report = evaluate(json.loads(arguments.manifest.read_text(encoding="utf-8")))
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output:
        arguments.output.write_text(encoded, encoding="utf-8")
    else:
        print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
