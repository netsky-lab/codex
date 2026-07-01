#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
prefix="${PREFIX:-$HOME/.local}"
install_name="${CODEX_CHANNELS_INSTALL_NAME:-codex-channels-v141}"
install_dir="${CODEX_CHANNELS_INSTALL_DIR:-$prefix/lib/$install_name}"
tmp_dir="${install_dir}.tmp.$$"
codex_home="${CODEX_HOME:-$HOME/.codex}"

require_path() {
  local path="$1"
  if [[ ! -e "$script_dir/$path" ]]; then
    echo "Missing release payload path: $path" >&2
    exit 1
  fi
}

require_path codex
require_path codex-resources
require_path plugins
require_path .agents

mkdir -p "$prefix/bin" "$(dirname "$install_dir")"
rm -rf "$tmp_dir"
mkdir -p "$tmp_dir"

cp "$script_dir/codex" "$tmp_dir/codex"
cp -R "$script_dir/codex-resources" "$tmp_dir/codex-resources"
cp -R "$script_dir/plugins" "$tmp_dir/plugins"
cp -R "$script_dir/.agents" "$tmp_dir/.agents"
chmod 0755 "$tmp_dir/codex"
if [[ -f "$tmp_dir/codex-resources/bwrap" ]]; then
  chmod 0755 "$tmp_dir/codex-resources/bwrap"
fi

rm -rf "$install_dir"
mv "$tmp_dir" "$install_dir"
ln -sf "$install_dir/codex" "$prefix/bin/$install_name"
ln -sf "$install_dir/codex" "$prefix/bin/codex-channels"

if [[ "${INSTALL_TELEGRAM_PLUGIN:-1}" != "0" ]]; then
  mkdir -p "$codex_home"
  export CODEX_HOME="$codex_home"
  "$prefix/bin/$install_name" features enable plugins
  "$prefix/bin/$install_name" plugin marketplace add "$install_dir"
  "$prefix/bin/$install_name" plugin add telegram-channel@codex-channels
fi

cat <<EOF
Installed Codex Channels:
  $prefix/bin/$install_name
  $prefix/bin/codex-channels

Telegram plugin marketplace source:
  $install_dir

Start example:
  TELEGRAM_BOT_TOKEN='...' \\
  TELEGRAM_ALLOWED_CHAT_IDS='5673369656' \\
  TELEGRAM_OFFSET_FILE="\$HOME/.codex/telegram-offset-bot.json" \\
  "$prefix/bin/$install_name" --yolo
EOF
