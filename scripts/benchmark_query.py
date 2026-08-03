#!/usr/bin/env python3
"""Run a bounded, disposable deterministic query-load benchmark.

Each iteration invokes ``cortana --offline eval``. The Rust evaluator creates its own
temporary SQLite index and deterministic embedder, so this benchmark never opens the
operator's configured index, contacts a source, or needs a model provider.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import math
import os
import shutil
import subprocess
import sys
import time
from collections.abc import Sequence
from pathlib import Path
from typing import Any


def percentile(values: Sequence[float], fraction: float) -> float:
    """Return the nearest-rank percentile for a non-empty sequence."""

    if not values:
        raise ValueError("percentile requires at least one value")
    if not 0 <= fraction <= 1:
        raise ValueError("percentile fraction must be between zero and one")
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, math.ceil(fraction * len(ordered)) - 1))
    return ordered[index]


def run_once(binary: str, timeout_seconds: float) -> dict[str, Any]:
    started = time.perf_counter()
    try:
        completed = subprocess.run(
            [binary, "--offline", "eval"],
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired:
        return {
            "passed": False,
            "timed_out": True,
            "latency_ms": round((time.perf_counter() - started) * 1000),
        }

    latency_ms = round((time.perf_counter() - started) * 1000)
    result: dict[str, Any] = {
        "passed": completed.returncode == 0,
        "timed_out": False,
        "latency_ms": latency_ms,
        "returncode": completed.returncode,
    }
    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError:
        result["error"] = "evaluation did not emit valid JSON"
    else:
        result["fixture_version"] = report.get("fixture_version")
        result["evaluation_passed"] = report.get("passed") is True
        result["metrics"] = report.get("metrics", {})
        result["passed"] = result["passed"] and result["evaluation_passed"]
    if completed.stderr.strip():
        result["stderr"] = completed.stderr.strip()[-1000:]
    return result


def summarize(
    results: Sequence[dict[str, Any]], iterations: int, concurrency: int
) -> dict[str, Any]:
    latencies = [float(result["latency_ms"]) for result in results if "latency_ms" in result]
    summary: dict[str, Any] = {
        "benchmark": "cortana-deterministic-eval",
        "isolated": True,
        "iterations": iterations,
        "concurrency": concurrency,
        "passed": len(results) == iterations
        and all(result.get("passed") is True for result in results),
        "results": list(results),
    }
    if latencies:
        summary["latency_ms"] = {
            "min": round(min(latencies)),
            "p50": round(percentile(latencies, 0.50)),
            "p95": round(percentile(latencies, 0.95)),
            "max": round(max(latencies)),
        }
    return summary


def parser() -> argparse.ArgumentParser:
    command = argparse.ArgumentParser(description=__doc__)
    command.add_argument(
        "--binary",
        default=None,
        help="cortana executable (default: CORTANA_BIN or the cortana on PATH)",
    )
    command.add_argument("--iterations", type=int, default=8)
    command.add_argument("--concurrency", type=int, default=2)
    command.add_argument("--timeout-seconds", type=float, default=30.0)
    command.add_argument(
        "--max-p95-ms",
        type=float,
        default=None,
        help="optional failure threshold for the measured p95 latency",
    )
    return command


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parser().parse_args(argv)
    if not 1 <= arguments.iterations <= 100:
        raise SystemExit("--iterations must be between 1 and 100")
    if not 1 <= arguments.concurrency <= 32:
        raise SystemExit("--concurrency must be between 1 and 32")
    if arguments.timeout_seconds <= 0:
        raise SystemExit("--timeout-seconds must be greater than zero")
    if arguments.max_p95_ms is not None and arguments.max_p95_ms <= 0:
        raise SystemExit("--max-p95-ms must be greater than zero")

    binary = arguments.binary or os.environ.get("CORTANA_BIN") or shutil.which("cortana")
    if not binary:
        raise SystemExit("cortana executable not found; pass --binary or set CORTANA_BIN")
    binary_path = str(Path(binary).expanduser())
    with concurrent.futures.ThreadPoolExecutor(max_workers=arguments.concurrency) as pool:
        results = list(
            pool.map(
                lambda _: run_once(binary_path, arguments.timeout_seconds),
                range(arguments.iterations),
            )
        )
    summary = summarize(results, arguments.iterations, arguments.concurrency)
    if arguments.max_p95_ms is not None:
        measured = summary.get("latency_ms", {}).get("p95")
        if measured is None or measured > arguments.max_p95_ms:
            summary["passed"] = False
            summary["latency_threshold_exceeded"] = True
    json.dump(summary, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
