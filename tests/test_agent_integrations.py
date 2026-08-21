"""Regression tests for the portable agent-skill installer."""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "scripts" / "install-agent-integrations.sh"


@pytest.mark.skipif(shutil.which("bash") is None, reason="agent installer requires bash")
def test_installer_accepts_current_codex_symlink(tmp_path: Path) -> None:
    """A managed Codex symlink must not make an otherwise successful install fail."""

    fake_binary = tmp_path / "bin" / "cortana"
    fake_binary.parent.mkdir()
    fake_binary.write_text("#!/bin/sh\nexit 0\n")
    fake_binary.chmod(0o755)
    config = tmp_path / "config.toml"
    config.write_text("[query]\n")

    agents_root = tmp_path / ".agents" / "skills"
    agents_skill = agents_root / "cortana"
    shutil.copytree(ROOT / "skills" / "cortana", agents_skill)
    codex_root = tmp_path / ".codex" / "skills"
    codex_root.mkdir(parents=True)
    (codex_root / "cortana").symlink_to(
        Path("../../.agents/skills/cortana"), target_is_directory=True
    )

    env = {
        **os.environ,
        "CORTANA_BINARY": str(fake_binary),
        "CORTANA_CONFIG": str(config),
        "CORTANA_SKILL_ROOTS": f"{codex_root}:{agents_root}",
    }
    result = subprocess.run(
        ["bash", str(INSTALLER)],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert (codex_root / "cortana").is_symlink()
    assert "already current" in result.stdout
