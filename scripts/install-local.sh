#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
install_prefix="${CORTANA_INSTALL_PREFIX:-$HOME/.local}"
config_root="${XDG_CONFIG_HOME:-$HOME/.config}/cortana"
data_root="${XDG_DATA_HOME:-$HOME/.local/share}/cortana"
bin_dir="$install_prefix/bin"
share_dir="$install_prefix/share/cortana"
web_dir="$share_dir/web"
venv_dir="$share_dir/venv"
config_path="${CORTANA_CONFIG:-$config_root/config.toml}"

for program in cargo bun uv; do
  command -v "$program" >/dev/null ||
    { echo "required program is missing: $program" >&2; exit 1; }
done

cd "$repo_dir"
cargo build --release --locked
bun install --frozen-lockfile
bun run build

install -d -m 0755 "$bin_dir" "$share_dir" "$config_root" "$data_root"
binary_source="$repo_dir/target/release/cortana"
binary_name="cortana"
if [[ ! -f "$binary_source" && -f "$repo_dir/target/release/cortana.exe" ]]; then
  binary_source="$repo_dir/target/release/cortana.exe"
  binary_name="cortana.exe"
fi
[[ -f "$binary_source" ]] || {
  echo "release binary is missing: $repo_dir/target/release/cortana[.exe]" >&2
  exit 1
}
binary_path="$bin_dir/$binary_name"
install -m 0755 "$binary_source" "$binary_path.new"
mv -f "$binary_path.new" "$binary_path"

web_stage="$share_dir/web.stage.$$"
install -d -m 0755 "$web_stage"
cp -R apps/web/dist/. "$web_stage/"
if [[ -d "$web_dir" ]]; then
  web_previous="$share_dir/web.previous.$$"
  mv "$web_dir" "$web_previous"
  mv "$web_stage" "$web_dir"
  rm -rf "$web_previous"
else
  mv "$web_stage" "$web_dir"
fi

uv venv --python 3.11 --allow-existing "$venv_dir"
if [[ -x "$venv_dir/bin/python" ]]; then
  venv_python="$venv_dir/bin/python"
elif [[ -x "$venv_dir/Scripts/python.exe" ]]; then
  venv_python="$venv_dir/Scripts/python.exe"
else
  echo "uv did not create a usable Python executable in $venv_dir" >&2
  exit 1
fi
if [[ "$venv_python" == */Scripts/python.exe ]]; then
  connector_command="$venv_dir/Scripts/cortana-connectors.exe"
else
  connector_command="$venv_dir/bin/cortana-connectors"
fi
uv pip install --python "$venv_python" ".[ingestion]"

if [[ ! -f "$config_path" ]]; then
  "$binary_path" --config "$config_path" init \
    --data-dir "$data_root" \
    --connector-command "$connector_command"
fi

if [[ "$(uname -s)" == "Darwin" && "${CORTANA_INSTALL_SERVICE:-1}" == "1" ]]; then
  if [[ "${CORTANA_ENABLE_SYNC_SERVICE:-0}" == "1" ]]; then
    "$binary_path" --config "$config_path" service install \
      --web-dir "$web_dir" \
      --working-directory "$share_dir" \
      --enable-sync-service
  else
    "$binary_path" --config "$config_path" service install \
      --web-dir "$web_dir" \
      --working-directory "$share_dir"
  fi
fi

if [[ "${CORTANA_INSTALL_AGENT_INTEGRATIONS:-0}" == "1" ]]; then
  CORTANA_BINARY="$binary_path" \
    CORTANA_CONFIG="$config_path" \
    "$repo_dir/scripts/install-agent-integrations.sh"
fi

echo "Cortana installed"
echo "  binary: $binary_path"
echo "  config: $config_path"
echo "  web:    $web_dir"
