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
if "sync" in sys.argv and os.environ.get("SMOKE_BINARY_FAIL_FIRST_SYNC"):
    marker = os.environ["SMOKE_BINARY_FAIL_FIRST_SYNC"]
    if not os.path.exists(marker):
        open(marker, "w", encoding="utf-8").close()
        print("temporarily unavailable", file=sys.stderr)
        sys.exit(1)
if "sync" in sys.argv and os.environ.get("SMOKE_BINARY_SYNC_ERROR"):
    print(os.environ["SMOKE_BINARY_SYNC_ERROR"], file=sys.stderr)
    sys.exit(1)
if os.environ.get("SMOKE_BINARY_ERROR"):
    print(os.environ["SMOKE_BINARY_ERROR"], file=sys.stderr)
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
    error: str = "",
    fail_first_sync: bool = False,
    sync_error: str = "",
) -> tuple[subprocess.CompletedProcess[str], Path]:
    log = tmp_path / "invocations.jsonl"
    smoke_tmpdir = tmp_path / "smoke-tmp"
    smoke_tmpdir.mkdir()
    env = os.environ.copy()
    env["SMOKE_BINARY_LOG"] = str(log)
    env["SMOKE_BINARY_EXIT"] = exit_code
    env["SMOKE_BINARY_ERROR"] = error
    env["SMOKE_BINARY_SYNC_ERROR"] = sync_error
    if fail_first_sync:
        env["SMOKE_BINARY_FAIL_FIRST_SYNC"] = str(tmp_path / "first-sync.marker")
    env["TMPDIR"] = str(smoke_tmpdir)
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


def test_bounded_trial_retries_once_after_a_transient_connector_failure(
    tmp_path: Path,
) -> None:
    _require_bash()
    result, log = _run_smoke(tmp_path, "--sync", fail_first_sync=True)
    assert result.returncode == 0, result.stderr
    assert "drive\tgoogle-drive\ttrue\tpassed\tpassed" in result.stdout
    drive_trials = [
        invocation
        for invocation in _invocations(log)
        if "sync" in invocation
        and invocation[invocation.index("sync") : invocation.index("sync") + 3]
        == ["sync", "--source", "drive"]
    ]
    assert len(drive_trials) == 2, drive_trials


def test_non_retryable_trial_failure_fails_fast(tmp_path: Path) -> None:
    _require_bash()
    result, log = _run_smoke(tmp_path, "--sync", sync_error="invalid_grant")
    assert result.returncode == 1
    assert "drive\tgoogle-drive\ttrue\tpassed\tfailed" in result.stdout
    assert "authorization denied after 1 attempt(s)" in result.stdout
    drive_trials = [
        invocation
        for invocation in _invocations(log)
        if "sync" in invocation
        and invocation[invocation.index("sync") : invocation.index("sync") + 3]
        == ["sync", "--source", "drive"]
    ]
    assert len(drive_trials) == 1, drive_trials


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


def test_google_token_refresh_failure_is_classified_as_authorization(
    tmp_path: Path,
) -> None:
    _require_bash()
    result, _ = _run_smoke(
        tmp_path,
        exit_code="1",
        error="Client error '400 Bad Request' for url 'https://oauth2.googleapis.com/token'",
    )
    assert result.returncode == 1
    assert (
        "drive\tgoogle-drive\ttrue\tfailed\tnot-requested\tvalidation: authorization denied"
        in result.stdout
    )


def test_missing_private_oauth_file_is_classified_as_credential_path(
    tmp_path: Path,
) -> None:
    _require_bash()
    result, _ = _run_smoke(
        tmp_path,
        exit_code="1",
        error="Discord OAuth client must be a regular, non-symlink file",
    )
    assert result.returncode == 1
    assert (
        "drive\tgoogle-drive\ttrue\tfailed\tnot-requested\tvalidation: credential or path missing"
        in result.stdout
    )


def test_temporary_diagnostics_are_removed_after_a_failure(tmp_path: Path) -> None:
    _require_bash()
    result, _ = _run_smoke(tmp_path, exit_code="1", error="connector failed")
    assert result.returncode == 1
    assert list((tmp_path / "smoke-tmp").iterdir()) == []
