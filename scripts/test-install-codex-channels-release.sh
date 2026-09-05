#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
test_root="$(mktemp -d)"
cleanup() {
  rm -rf -- "$test_root"
}
trap cleanup EXIT

fixture="$test_root/bundle"
prefix="$test_root/prefix"
mkdir -p \
  "$fixture/bin" \
  "$fixture/codex-resources" \
  "$fixture/codex-resources/zsh/bin" \
  "$fixture/codex-path" \
  "$fixture/plugins/telegram-channel/.codex-plugin" \
  "$fixture/.agents/plugins"

cp "$repo_root/scripts/install-codex-channels-release.sh" "$fixture/install.sh"
for executable in \
  bin/codex \
  bin/codex-code-mode-host \
  bin/codex-responses-api-proxy \
  codex-resources/bwrap \
  codex-resources/zsh/bin/zsh \
  codex-path/rg
do
  printf '#!/usr/bin/env bash\nexit 0\n' > "$fixture/$executable"
  chmod 0755 "$fixture/$executable"
done
printf '{"layoutVersion":1,"version":"0.153.4"}\n' > "$fixture/codex-package.json"
printf '{"name":"telegram-channel"}\n' > "$fixture/plugins/telegram-channel/.codex-plugin/plugin.json"
printf '{"name":"codex-channels"}\n' > "$fixture/.agents/plugins/marketplace.json"

PREFIX="$prefix" HOME="$test_root/home" "$fixture/install.sh" > "$test_root/install.out"

install_dir="$prefix/lib/codex-channels/codex-channels-v0.153.4"
test -x "$install_dir/bin/codex"
test -x "$install_dir/bin/codex-code-mode-host"
test -x "$install_dir/bin/codex-responses-api-proxy"
test -x "$install_dir/codex-resources/bwrap"
test -x "$install_dir/codex-resources/zsh/bin/zsh"
test -x "$install_dir/codex-path/rg"
test -L "$prefix/bin/codex-channels"
test -L "$prefix/bin/codex-channels-v0.153.4"
test ! -e "$prefix/bin/codex"
test ! -e "$test_root/home/.codex"

if PREFIX="$prefix" HOME="$test_root/home" "$fixture/install.sh" > /dev/null 2>&1; then
  echo "second install unexpectedly replaced the existing version" >&2
  exit 1
fi

if PREFIX="$test_root/reserved-prefix" CODEX_CHANNELS_INSTALL_NAME=codex \
  HOME="$test_root/home" "$fixture/install.sh" > /dev/null 2>&1; then
  echo "reserved codex install name was unexpectedly accepted" >&2
  exit 1
fi
test ! -e "$test_root/reserved-prefix/bin/codex"

dangling_prefix="$test_root/dangling-prefix"
dangling_root="$dangling_prefix/lib/codex-channels"
mkdir -p "$dangling_root"
ln -s "$test_root/missing-install-target" \
  "$dangling_root/codex-channels-v0.153.4"
if PREFIX="$dangling_prefix" HOME="$test_root/home" \
  "$fixture/install.sh" > /dev/null 2>&1; then
  echo "dangling install destination was unexpectedly replaced" >&2
  exit 1
fi
test -L "$dangling_root/codex-channels-v0.153.4"
test ! -e "$dangling_prefix/bin/codex-channels"

relative_work="$test_root/relative-work"
mkdir -p "$relative_work"
relative_work="$(cd -- "$relative_work" && pwd -P)"
(
  cd "$relative_work"
  PREFIX=relative-prefix HOME="$test_root/home" "$fixture/install.sh" > /dev/null
)
relative_prefix="$relative_work/relative-prefix"
test -x "$relative_prefix/bin/codex-channels"
test "$(readlink "$relative_prefix/bin/codex-channels")" = \
  "$relative_prefix/lib/codex-channels/codex-channels-v0.153.4/bin/codex"

custom_work="$test_root/custom-work"
mkdir -p "$custom_work"
custom_work="$(cd -- "$custom_work" && pwd -P)"
(
  cd "$custom_work"
  PREFIX=relative-prefix CODEX_CHANNELS_INSTALL_ROOT=relative-install-root \
    HOME="$test_root/home" CODEX_CHANNELS_INSTALL_NAME=custom-relative \
    "$fixture/install.sh" > /dev/null
)
test -x "$custom_work/relative-prefix/bin/codex-channels"
test "$(readlink "$custom_work/relative-prefix/bin/codex-channels")" = \
  "$custom_work/relative-install-root/custom-relative/bin/codex"

checksum_source="$test_root/checksum-source"
checksum_verify="$test_root/checksum-verify"
mkdir -p "$checksum_source" "$checksum_verify"
printf 'portable bundle fixture\n' > "$checksum_source/codex-channels-test.tar.gz"
(
  cd "$checksum_source"
  sha256sum codex-channels-test.tar.gz > codex-channels-test.tar.gz.sha256
)
cp "$checksum_source/codex-channels-test.tar.gz" \
  "$checksum_source/codex-channels-test.tar.gz.sha256" "$checksum_verify/"
(
  cd "$checksum_verify"
  sha256sum -c codex-channels-test.tar.gz.sha256 > /dev/null
)

echo "codex-channels installer fixture test passed"
