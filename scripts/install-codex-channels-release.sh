#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
invocation_dir="$(pwd -P)"
prefix_input="${PREFIX:-$HOME/.local}"
install_name="${CODEX_CHANNELS_INSTALL_NAME:-codex-channels-v0.153.4}"
codex_home="${CODEX_HOME:-$HOME/.codex}"

if [[ ! "$install_name" =~ ^[A-Za-z0-9._-]+$ || "$install_name" == "." || "$install_name" == ".." || "$install_name" == "codex" ]]; then
  echo "Invalid CODEX_CHANNELS_INSTALL_NAME: $install_name" >&2
  exit 1
fi

case "$prefix_input" in
  /*) prefix="$prefix_input" ;;
  *) prefix="$invocation_dir/$prefix_input" ;;
esac

if [[ -n "${CODEX_CHANNELS_INSTALL_ROOT:-}" ]]; then
  case "$CODEX_CHANNELS_INSTALL_ROOT" in
    /*) install_root="$CODEX_CHANNELS_INSTALL_ROOT" ;;
    *) install_root="$invocation_dir/$CODEX_CHANNELS_INSTALL_ROOT" ;;
  esac
else
  install_root="$prefix/lib/codex-channels"
fi
install_dir="$install_root/$install_name"

require_file() {
  local relative_path="$1"
  if [[ ! -f "$script_dir/$relative_path" ]]; then
    echo "Missing release payload file: $relative_path" >&2
    exit 1
  fi
}

require_file codex-package.json
require_file bin/codex
require_file bin/codex-code-mode-host
require_file bin/codex-responses-api-proxy
require_file codex-resources/bwrap
require_file codex-resources/zsh/bin/zsh
require_file codex-path/rg
require_file plugins/telegram-channel/.codex-plugin/plugin.json
require_file .agents/plugins/marketplace.json

if [[ -e "$install_dir" || -L "$install_dir" ]]; then
  echo "Install destination already exists: $install_dir" >&2
  echo "Choose another CODEX_CHANNELS_INSTALL_NAME or remove that exact version explicitly." >&2
  exit 1
fi

mkdir -p "$prefix/bin" "$install_root"
stage_dir="$(mktemp -d "$install_root/.${install_name}.install.XXXXXX")"
cleanup_stage() {
  if [[ -d "$stage_dir" ]]; then
    rm -rf -- "$stage_dir"
  fi
}
trap cleanup_stage EXIT

cp -R "$script_dir/." "$stage_dir/"
chmod 0755 \
  "$stage_dir/bin/codex" \
  "$stage_dir/bin/codex-code-mode-host" \
  "$stage_dir/bin/codex-responses-api-proxy" \
  "$stage_dir/codex-resources/bwrap" \
  "$stage_dir/codex-resources/zsh/bin/zsh" \
  "$stage_dir/codex-path/rg"
mv "$stage_dir" "$install_dir"
trap - EXIT

ln -sfn "$install_dir/bin/codex" "$prefix/bin/$install_name"
ln -sfn "$install_dir/bin/codex" "$prefix/bin/codex-channels"

if [[ "${INSTALL_TELEGRAM_PLUGIN:-0}" == "1" ]]; then
  mkdir -p "$codex_home"
  export CODEX_HOME="$codex_home"
  "$prefix/bin/codex-channels" features enable plugins
  "$prefix/bin/codex-channels" plugin marketplace add "$install_dir"
  "$prefix/bin/codex-channels" plugin add telegram-channel@codex-channels
fi

cat <<EOF
Installed Codex Channels 0.153.4:
  $prefix/bin/codex-channels
  $prefix/bin/$install_name

Bundle directory:
  $install_dir

Telegram plugin installation was ${INSTALL_TELEGRAM_PLUGIN:-0}.
Set INSTALL_TELEGRAM_PLUGIN=1 when running this installer to register it.
EOF
