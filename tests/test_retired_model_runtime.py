"""Keep retired model identifiers out of shipped Cortana runtime paths."""

from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).parents[1]
RUNTIME_PATHS = (
    ROOT / "src",
    ROOT / "apps",
    ROOT / "scripts",
    ROOT / "Cargo.toml",
    ROOT / "pyproject.toml",
    ROOT / "package.json",
    ROOT / "config.example.toml",
    ROOT / ".env.example",
)
RETIRED_IDENTIFIERS = (
    "gpt-5.3-codex-spark",
    "spark_provider",
    "spark-model",
    "spark_model",
)


def _runtime_files() -> list[Path]:
    tracked = subprocess.run(
        [
            "git",
            "ls-files",
            "-z",
            "--",
            *(str(path.relative_to(ROOT)) for path in RUNTIME_PATHS),
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    return [
        ROOT / relative
        for relative in tracked.stdout.decode("utf-8").split("\0")
        if relative and (ROOT / relative).is_file()
    ]


def test_retired_model_identifiers_are_absent_from_runtime_paths() -> None:
    findings: list[str] = []
    for path in _runtime_files():
        try:
            content = path.read_bytes().lower()
        except OSError as error:
            findings.append(f"{path.relative_to(ROOT)} could not be read: {error}")
            continue
        for identifier in RETIRED_IDENTIFIERS:
            if identifier.encode("ascii") in content:
                findings.append(f"{path.relative_to(ROOT)} contains {identifier}")

    assert not findings, "retired model/provider identifiers found:\n" + "\n".join(findings)
