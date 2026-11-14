#!/usr/bin/env bash
# Runs on-screen (instrumented) Android tests on a headless emulator. Needs a
# Linux machine with KVM, ANDROID_HOME with the pinned SDK and NDK, cargo-ndk.
# The app is rebuilt for the emulator's x86_64 processor; that build is only
# for this test and proves nothing about the S23, its pen or its screen.
# Usage: tools/ci/android-emulator.sh [test class ...]   (default: the list below)
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK directory}"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$ANDROID_NDK"
sdkmanager="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
avdmanager="$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager"
adb="$ANDROID_HOME/platform-tools/adb"
if ! command -v python >/dev/null 2>&1; then
  shim="$(mktemp -d)"; ln -s "$(command -v "$PYTHON")" "$shim/python"; export PATH="$shim:$PATH"
fi

if [ "$#" -eq 0 ]; then
  set -- com.visualworkbench.android.StartupInstrumentedTest \
    com.visualworkbench.android.EditorSurfaceInstrumentedTest \
    com.visualworkbench.android.EditorCoreInstrumentedTest \
    com.visualworkbench.android.editor.MarkerDrawingInstrumentedTest
fi
classes="$(IFS=,; echo "$*")"

step "emulator packages"
(yes 2>/dev/null || true) | "$sdkmanager" --licenses >/dev/null
"$sdkmanager" --install platform-tools emulator "$ANDROID_EMULATOR_IMAGE" >/dev/null

step "rust natives (host, Android arm64 for the build checks, Android x86_64 for the emulator)"
rustup target add x86_64-linux-android
cargo build --locked -p vw-ffi -p vw-host-ffi --features vw-ffi/bindgen --lib --bin vw-bindgen
cargo ndk -t arm64-v8a -t x86_64 --platform 29 -o target/android-jni build --locked -p vw-ffi

step "start emulator"
echo no | "$avdmanager" create avd --force -n vw-ci -k "$ANDROID_EMULATOR_IMAGE" -d pixel_6 >/dev/null
"$ANDROID_HOME/emulator/emulator" -avd vw-ci -no-window -no-audio -no-boot-anim -no-snapshot \
  -gpu swiftshader_indirect -memory 4096 -cores 4 -camera-back none >"${TMPDIR:-/tmp}/vw-emulator.log" 2>&1 &
emulator_pid=$!
trap '"$adb" emu kill >/dev/null 2>&1 || true; kill "$emulator_pid" 2>/dev/null || true' EXIT
booted=""
for _ in $(seq 1 150); do
  if [ "$("$adb" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; then booted=1; break; fi
  kill -0 "$emulator_pid" 2>/dev/null || { tail -n 40 "${TMPDIR:-/tmp}/vw-emulator.log"; echo "emulator exited"; exit 1; }
  sleep 4
done
[ -n "$booted" ] || { tail -n 40 "${TMPDIR:-/tmp}/vw-emulator.log"; echo "emulator did not boot in 10 minutes"; exit 1; }
for scale in window_animation_scale transition_animation_scale animator_duration_scale; do
  "$adb" shell settings put global "$scale" 0
done
"$adb" shell input keyevent 82 || true

step "instrumented tests: $classes"
bash apps/gradlew -p apps :android:connectedDebugAndroidTest -PvwEmulatorAbi=x86_64 \
  "-Pandroid.testInstrumentationRunnerArguments.class=$classes" \
  -x :desktop:packageConnectionRuntime --console=plain --no-daemon --no-parallel --max-workers="${VW_GRADLE_WORKERS:-2}"
