#!/usr/bin/env bash
# Kotlin/Compose checks on Linux, following the repository's recorded recipe:
# build the Rust natives and bindings generator, build the Android library with
# the NDK, then run Gradle. Needs ANDROID_HOME with the pinned platform,
# build-tools and NDK (tools/ci/install-tools.sh android) and cargo-ndk.
# Usage: tools/ci/kotlin-linux.sh [gradle tasks...]   (default: the full check set)
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK directory}"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$ANDROID_NDK"
# Gradle exec tasks call "python"; make sure that name resolves.
if ! command -v python >/dev/null 2>&1; then
  shim="$(mktemp -d)"; ln -s "$(command -v "$PYTHON")" "$shim/python"; export PATH="$shim:$PATH"
fi

step "rust natives and bindings generator (host)"
cargo build --locked -p vw-ffi -p vw-host-ffi --features vw-ffi/bindgen --lib --bin vw-bindgen
step "rust native library (Android arm64)"
cargo ndk -t arm64-v8a --platform 29 -o target/android-jni build --locked -p vw-ffi

if [ "$#" -eq 0 ]; then
  set -- checkDependencyLicenses \
    :shared:allTests :android:testDebugUnitTest :desktop:test :pen-probe:testDebugUnitTest \
    :android:lintDebug :shared:check :desktop:check
fi
step "gradle: $*"
# The connection-runtime packaging task needs Windows binaries; every recorded
# Linux run excludes it.
bash apps/gradlew -p apps "$@" -x :desktop:packageConnectionRuntime \
  --console=plain --no-daemon --no-parallel --max-workers="${VW_GRADLE_WORKERS:-2}"
