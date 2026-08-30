from __future__ import annotations

import importlib.util
import json
from pathlib import Path
from types import ModuleType


def load_evaluator() -> ModuleType:
    path = Path("scripts/evaluate-code-retrieval.py")
    spec = importlib.util.spec_from_file_location("evaluate_code_retrieval", path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_code_strategy_decision_is_measured_and_activation_gated() -> None:
    module = load_evaluator()
    manifest = json.loads(Path("eval/code-retrieval-v1.json").read_text(encoding="utf-8"))
    report = module.evaluate(manifest)
    selected = report["results"][report["selected_strategy"]]
    assert selected["recall_at_3"] == 1.0
    assert selected["mrr"] >= report["results"]["shared_text_embedding"]["mrr"]
    assert selected["span_correctness"] == 1.0
    assert selected["cache_reuse"] == 1.0
    assert selected["index_bytes"] > 0
    assert report["resource_budget_pass"]
    assert report["migration_cost"]["documents_scanned"] == len(manifest["documents"])
    assert report["migration_cost"]["documents_reembedded"] == 0
    assert report["activation"]["approved_roots_required"]
    assert report["activation"]["sampled_non_reconciling_trial_required"]
    assert report["activation"]["second_embedding_generation"] is False
