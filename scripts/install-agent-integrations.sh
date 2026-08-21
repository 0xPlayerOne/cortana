#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
skill_source="$repo_dir/skills/cortana"
binary_path="${CORTANA_BINARY:-${CORTANA_INSTALL_PREFIX:-$HOME/.local}/bin/cortana}"
config_path="${CORTANA_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/cortana/config.toml}"
# Install into the current Codex/agent skill roots by default. Legacy Hermes and
# OpenCode locations are opt-in so an install cannot mutate unrelated harnesses.
skill_roots="${CORTANA_SKILL_ROOTS:-$HOME/.codex/skills:$HOME/.agents/skills}"

if [[ ! -x "$binary_path" && -x "${binary_path}.exe" ]]; then
  binary_path="${binary_path}.exe"
fi
if [[ ! -x "$binary_path" ]]; then
  echo "Cortana binary is not executable: $binary_path" >&2
  exit 1
fi

if [[ ! -f "$config_path" ]]; then
  echo "Cortana config does not exist: $config_path" >&2
  exit 1
fi

if [[ ! -f "$skill_source/SKILL.md" ]]; then
  echo "Cortana skill is missing: $skill_source/SKILL.md" >&2
  exit 1
fi

IFS=: read -r -a roots <<< "$skill_roots"
for root in "${roots[@]}"; do
  [[ -n "$root" ]] || continue
  destination="$root/cortana"
  install -d -m 0755 "$root"
  stage="$(mktemp -d "$root/.cortana.stage.XXXXXX")"
  previous_dir="$(mktemp -d "$root/.cortana.previous.XXXXXX")"
  previous="$previous_dir/cortana"
  moved_previous=0

  rollback() {
    if ((moved_previous)) && [[ ! -e "$destination" && ! -L "$destination" ]] && [[ -e "$previous" ]]; then
      mv "$previous" "$destination" || true
    fi
    rm -rf -- "$stage" "$previous_dir"
  }

  if ! cp -R "$skill_source/." "$stage/" ||
    ! find "$stage" -type d -exec chmod 0755 {} + ||
    ! find "$stage" -type f -exec chmod 0644 {} +; then
    rollback
    echo "Unable to stage Cortana skill for $root" >&2
    exit 1
  fi

  if [[ -L "$destination" ]]; then
    if diff -qr "$skill_source" "$destination" >/dev/null 2>&1; then
      rollback
      echo "Cortana skill already current (symlink): $destination"
      continue
    fi
    rollback
    echo "Refusing to replace symlinked Cortana skill: $destination" >&2
    exit 1
  fi
  if [[ -e "$destination" ]]; then
    if ! mv "$destination" "$previous"; then
      rollback
      echo "Unable to preserve existing Cortana skill: $destination" >&2
      exit 1
    fi
    moved_previous=1
  fi
  if ! mv "$stage" "$destination"; then
    rollback
    echo "Unable to install Cortana skill: $destination" >&2
    exit 1
  fi
  rm -rf -- "$previous_dir"
  echo "Installed Cortana skill: $destination"
done

echo "Configure agent MCP clients with:"
echo "  command: $binary_path"
echo "  args:    --config $config_path mcp"
echo "For shared agents, append: --token-env <configured [[auth.tokens]] environment name>"
