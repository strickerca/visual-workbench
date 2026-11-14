#!/usr/bin/env bash
# Installs the pinned check tools on a Linux machine (cloud session or CI runner).
# Usage: tools/ci/install-tools.sh [deny] [gitleaks] [ndk] [android]
# With no arguments installs all four. Idempotent: an installed pinned version is kept.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck disable=SC1091
. "$root/tools/ci/pins.env"
bin="${VW_CI_BIN:-$HOME/.local/bin}"
mkdir -p "$bin"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fetch() { curl -fsSL --retry 3 --max-time 600 -o "$2" "$1"; }
check256() { echo "$1  $2" | sha256sum -c - >/dev/null; }

install_deny() {
  if "$bin/cargo-deny" --version 2>/dev/null | grep -q "$CARGO_DENY_VERSION"; then return; fi
  local name="cargo-deny-$CARGO_DENY_VERSION-x86_64-unknown-linux-musl"
  fetch "https://github.com/EmbarkStudios/cargo-deny/releases/download/$CARGO_DENY_VERSION/$name.tar.gz" "$work/deny.tgz"
  check256 "$CARGO_DENY_LINUX_SHA256" "$work/deny.tgz"
  tar -xzf "$work/deny.tgz" -C "$work"
  install -m 0755 "$work/$name/cargo-deny" "$bin/cargo-deny"
}

install_gitleaks() {
  if "$bin/gitleaks" version 2>/dev/null | grep -q "$GITLEAKS_VERSION"; then return; fi
  fetch "https://github.com/gitleaks/gitleaks/releases/download/v$GITLEAKS_VERSION/gitleaks_${GITLEAKS_VERSION}_linux_x64.tar.gz" "$work/gitleaks.tgz"
  check256 "$GITLEAKS_LINUX_SHA256" "$work/gitleaks.tgz"
  tar -xzf "$work/gitleaks.tgz" -C "$work" gitleaks
  install -m 0755 "$work/gitleaks" "$bin/gitleaks"
}

install_ndk() {
  if cargo install --list 2>/dev/null | grep -q "^cargo-ndk v$CARGO_NDK_VERSION"; then return; fi
  cargo install --locked "cargo-ndk@$CARGO_NDK_VERSION"
}

install_android() {
  : "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK directory}"
  local sdkmanager="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
  if [ ! -x "$sdkmanager" ]; then
    fetch "https://dl.google.com/android/repository/$ANDROID_CMDLINE_TOOLS_ZIP" "$work/cmdline.zip"
    echo "$ANDROID_CMDLINE_TOOLS_SHA1  $work/cmdline.zip" | sha1sum -c - >/dev/null
    mkdir -p "$ANDROID_HOME/cmdline-tools"
    unzip -q "$work/cmdline.zip" -d "$work/cmdline"
    rm -rf "$ANDROID_HOME/cmdline-tools/latest"
    mv "$work/cmdline/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest"
  fi
  # The licence prompt reads "y" answers; sdkmanager closing the pipe is expected.
  (yes 2>/dev/null || true) | "$sdkmanager" --licenses >/dev/null
  "$sdkmanager" --install "platforms;$ANDROID_PLATFORM" "build-tools;$ANDROID_BUILD_TOOLS" "ndk;$ANDROID_NDK" >/dev/null
  test -d "$ANDROID_HOME/ndk/$ANDROID_NDK"
}

if [ "$#" -eq 0 ]; then set -- deny gitleaks ndk android; fi
for tool in "$@"; do
  case "$tool" in
    deny) install_deny ;;
    gitleaks) install_gitleaks ;;
    ndk) install_ndk ;;
    android) install_android ;;
    *) echo "unknown tool: $tool" >&2; exit 2 ;;
  esac
  echo "ok: $tool"
done
