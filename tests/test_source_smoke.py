"""Deterministic regression tests for scripts/source-smoke.sh.

The smoke probe invokes a fake Cortana binary that records its arguments, so
the tests assert the generated commands without touching a real connector,
index, or user configuration. Filesystem/code validations must pass the
explicit `--sample` flag (bounded sample) while connector sources keep
ordinary fail-closed validation, and every `--sync` trial must stay equally
bounded and non-reconciling.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

SECRET_TOKEN = "super-secret-token-value"

FAKE_BINARY = """\
#!/usr/bin/env python3
import json
import os
import sys

with open(os.environ["SMOKE_BINARY_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps(sys.argv[1:]) + "\\n")
sys.exit(int(os.environ.get("SMOKE_BINARY_EXIT", "0")))
"""

CONFIG_TOML = """\
sources = [
  {{ name = "work-code", kind = "filesystem", enabled = true, root = "/tmp/code", token = "{token}" }},
  {{ name = "drive", kind = "google-drive", enabled = true, token = "{token}" }},
]
"""

SMOKE_SCRIPT = Path(__file__).parents[1] / "scripts" / "source-smoke.sh"


def _require_bash() -> None:
    if os.name == "nt" or shutil.which("bash") is None:
        pytest.skip("source-smoke.sh requires bash")


def _write_fake_binary(tmp_path: Path) -> Path:
    binary = tmp_path / "fake-cortana"
    binary.write_text(FAKE_BINARY, encoding="utf-8")
    binary.chmod(0o755)
    return binary


def _write_config(tmp_path: Path) -> Path:
    config = tmp_path / "config.toml"
    config.write_text(CONFIG_TOML.format(token=SECRET_TOKEN), encoding="utf-8")
    return config


def _run_smoke(
    tmp_path: Path,
    *extra: str,
    exit_code: str = "0",
) -> tuple[subprocess.CompletedProcess[str], Path]:
    log = tmp_path / "invocations.jsonl"
    env = os.environ.copy()
    env["SMOKE_BINARY_LOG"] = str(log)
    env["SMOKE_BINARY_EXIT"] = exit_code
    result = subprocess.run(
        [
            "bash",
            str(SMOKE_SCRIPT),
            "--config",
            str(_write_config(tmp_path)),
            "--binary",
            str(_write_fake_binary(tmp_path)),
            *extra,
        ],
        capture_output=True,
        text=True,
        env=env,
        check=False,
    )
    return result, log


def _invocations(log: Path) -> list[list[str]]:
    if not log.exists():
        return []
    return [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]


def _find(invocations: list[list[str]], *tokens: str) -> list[str]:
    for invocation in invocations:
        for index in range(len(invocation) - len(tokens) + 1):
            if invocation[index : index + len(tokens)] == list(tokens):
                return invocation
    raise AssertionError(f"invocation {tokens!r} was not generated: {invocations}")


def _budget(invocation: list[str]) -> tuple[str, str, str]:
    def value(flag: str) -> str:
        index = invocation.index(flag)
        return invocation[index + 1]

    return value("--max-documents"), value("--max-bytes"), value("--max-seconds")


def test_filesystem_validation_samples_and_trials_stay_equally_bounded(
    tmp_path: Path,
) -> None:
    _require_bash()
    result, log = _run_smoke(
        tmp_path,
        "--sync",
        "--include-filesystem",
        "--max-documents",
        "7",
        "--max-bytes",
        "12345",
        "--max-seconds",
        "33",
    )
    assert result.returncode == 0, result.stderr
    assert "source smoke passed" in result.stdout

    invocations = _invocations(log)
    budget = ("7", "12345", "33")
    filesystem_validation = _find(invocations, "validate-source", "work-code")
    assert "--sample" in filesystem_validation, filesystem_validation
    connector_validation = _find(invocations, "validate-source", "drive")
    assert "--sample" not in connector_validation, connector_validation
    assert _budget(filesystem_validation) == budget
    assert _budget(connector_validation) == budget

    # --sync --include-filesystem runs the equally bounded non-reconciling
    # trial for the sampled filesystem source and for connector sources.
    filesystem_trial = _find(invocations, "sync", "--source", "work-code")
    connector_trial = _find(invocations, "sync", "--source", "drive")
    for trial in (filesystem_trial, connector_trial):
        assert "--no-reconcile" in trial, trial
        assert "--require-validation" in trial, trial
        assert _budget(trial) == budget, trial
    assert "work-code\tfilesystem\ttrue\tpassed\tpassed" in result.stdout
    assert "drive\tgoogle-drive\ttrue\tpassed\tpassed" in result.stdout

    # The probe never reads or prints token values.
    assert SECRET_TOKEN not in result.stdout
    assert SECRET_TOKEN not in result.stderr
    assert all(
        SECRET_TOKEN not in argument for invocation in invocations for argument in invocation
    )


def test_filesystem_trials_require_include_filesystem(tmp_path: Path) -> None:
    _require_bash()
    result, log = _run_smoke(tmp_path, "--sync")
    assert result.returncode == 0, result.stderr
    assert "work-code\tfilesystem\ttrue\tpassed\tskipped-filesystem" in result.stdout
    assert "filesystem trial requires --include-filesystem" in result.stdout
    invocations = _invocations(log)
    assert all(
        not ("sync" in invocation and "work-code" in invocation) for invocation in invocations
    ), invocations
    _find(invocations, "sync", "--source", "drive")


def test_failed_validation_fails_closed_and_blocks_trials(tmp_path: Path) -> None:
    _require_bash()
    result, log = _run_smoke(
        tmp_path,
        "--sync",
        "--include-filesystem",
        exit_code="1",
    )
    assert result.returncode == 1
    assert "source smoke completed with 2 failure(s)" in result.stderr
    assert "work-code\tfilesystem\ttrue\tfailed\tskipped-validation" in result.stdout
    assert "drive\tgoogle-drive\ttrue\tfailed\tskipped-validation" in result.stdout
    assert all("sync" not in invocation for invocation in _invocations(log))
