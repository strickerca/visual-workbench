#!/usr/bin/env bash
# Source this file in every worker command shell; exports do not cross processes.
_vw_setup_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
export VW_TOOLS_ROOT="${VW_TOOLS_ROOT:-/workspace/.vw-tools}"
export CARGO_HOME="$VW_TOOLS_ROOT/cargo"
export RUSTUP_HOME="$VW_TOOLS_ROOT/rustup"
export GRADLE_USER_HOME="$VW_TOOLS_ROOT/gradle"
export npm_config_cache="$VW_TOOLS_ROOT/npm-cache"
export CARGO_BUILD_JOBS=2
export ANDROID_USER_HOME="$VW_TOOLS_ROOT/android-user"
export JAVA_HOME="$VW_TOOLS_ROOT/jdk-root/usr/lib/jvm/java-21-openjdk-amd64"
export ANDROID_HOME="$VW_TOOLS_ROOT/android-sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export PATH="$VW_TOOLS_ROOT/node-v24.21.0-linux-x64/bin:$CARGO_HOME/bin:$JAVA_HOME/bin:$ANDROID_HOME/cmdline-tools/22.0/bin:$ANDROID_HOME/build-tools/36.1.0:$PATH"
mkdir -p "$ANDROID_USER_HOME" "$GRADLE_USER_HOME" "$npm_config_cache"
# Keep CARGO_TARGET_DIR unset: each worktree owns its default target directory.
python3 "$_vw_setup_dir/proxy.py"
unset _vw_setup_dir
