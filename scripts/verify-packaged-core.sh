#!/usr/bin/env bash
set -euo pipefail

binary="${1:-}"
if [[ -z "$binary" || ! -x "$binary" ]]; then
  echo "usage: $0 PATH_TO_PACKAGED_CORTANA_BINARY" >&2
  exit 2
fi

timeout_seconds="${CORTANA_PACKAGED_EVAL_TIMEOUT_SECONDS:-60}"
if ! [[ "$timeout_seconds" =~ ^[1-9][0-9]*$ ]] || ((timeout_seconds > 60)); then
  echo "CORTANA_PACKAGED_EVAL_TIMEOUT_SECONDS must be a positive integer no greater than 60" >&2
  exit 2
fi

python3 - "$binary" "$timeout_seconds" <<'PY'
import json
import subprocess
import sys
import tempfile
from pathlib import Path

binary = Path(sys.argv[1])
timeout = float(sys.argv[2])
with tempfile.TemporaryDirectory(prefix="cortana-packaged-eval.") as directory:
    config = Path(directory) / "config.toml"
    config.write_text("[query]\n", encoding="utf-8")
    command = [str(binary), "--config", str(config), "--offline", "eval"]
    try:
        result = subprocess.run(command, check=False, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as error:
        raise SystemExit(f"packaged core offline evaluation timed out after {int(timeout)}s") from error
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        suffix = f": {detail[-500:]}" if detail else ""
        raise SystemExit(f"packaged core offline evaluation failed (exit {result.returncode}){suffix}")
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise SystemExit(f"packaged core evaluation did not emit JSON: {error}") from error
    if report.get("passed") is not True:
        raise SystemExit("packaged core offline evaluation did not pass")
    print(f"verified packaged core offline evaluation within {int(timeout)}s")
PY
