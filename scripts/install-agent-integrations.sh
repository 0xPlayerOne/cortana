#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
skill_source="$repo_dir/skills/cortana"
binary_path="${CORTANA_BINARY:-${CORTANA_INSTALL_PREFIX:-$HOME/.local}/bin/cortana}"
config_path="${CORTANA_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/cortana/config.toml}"
skill_roots="${CORTANA_SKILL_ROOTS:-$HOME/.codex/skills:$HOME/.hermes/skills:$HOME/.config/opencode/skills:$HOME/.agents/skills}"

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
  stage="$root/.cortana.stage.$$"
  previous="$root/.cortana.previous.$$"

  install -d -m 0755 "$root" "$stage"
  cp -R "$skill_source/." "$stage/"
  find "$stage" -type d -exec chmod 0755 {} +
  find "$stage" -type f -exec chmod 0644 {} +

  if [[ -e "$destination" ]]; then
    mv "$destination" "$previous"
  fi
  mv "$stage" "$destination"
  rm -rf "$previous"
  echo "Installed Cortana skill: $destination"
done

echo "Configure agent MCP clients with:"
echo "  command: $binary_path"
echo "  args:    --config $config_path mcp"
echo "For shared agents, append: --token-env <configured [[auth.tokens]] environment name>"
