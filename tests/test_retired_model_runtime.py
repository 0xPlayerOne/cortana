"""Keep retired model identifiers out of shipped Cortana runtime paths."""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).parents[1]
RUNTIME_PATHS = (
    ROOT / "src",
    ROOT / "apps",
    ROOT / "scripts",
    ROOT / "Cargo.toml",
    ROOT / "pyproject.toml",
    ROOT / "package.json",
)
RETIRED_IDENTIFIERS = (
    "gpt-5.3-codex-spark",
    "spark_provider",
    "spark-model",
    "spark_model",
)


def _runtime_files() -> list[Path]:
    files: list[Path] = []
    for path in RUNTIME_PATHS:
        if path.is_file():
            files.append(path)
        elif path.is_dir():
            files.extend(
                candidate
                for candidate in path.rglob("*")
                if candidate.is_file()
                and not set(candidate.relative_to(ROOT).parts).intersection(
                    {".git", "node_modules", "target", "dist"}
                )
            )
    return files


def test_retired_model_identifiers_are_absent_from_runtime_paths() -> None:
    findings: list[str] = []
    for path in _runtime_files():
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for identifier in RETIRED_IDENTIFIERS:
            if identifier in text.lower():
                findings.append(f"{path.relative_to(ROOT)} contains {identifier}")

    assert not findings, "retired model/provider identifiers found:\n" + "\n".join(findings)
