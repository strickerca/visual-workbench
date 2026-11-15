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
# The tool that creates the emulator and the emulator itself must agree on where it lives.
export ANDROID_AVD_HOME="${ANDROID_AVD_HOME:-$HOME/.android/avd}"
mkdir -p "$ANDROID_AVD_HOME"
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
# A fresh emulator shows a one-time "Viewing full screen" panel over the first
# full-screen app; that belongs to the emulator, not to the app.
"$adb" shell settings put secure immersive_mode_confirmations confirmed || true
# The on-screen tests write the canvas's lifecycle to the log; keep enough of it
# that the lines of an early test are still there when the run ends.
"$adb" logcat -G 16M || true

# On a software-rendered emulator the system's own launcher is slow enough to
# get an "isn't responding" dialog. It stays in front for the whole run and
# dims the screen to 40 %, which fails every screen-pixel check although the
# app drew correctly (seen in run 9: "Application Not Responding:
# com.google.android.apps.nexuslauncher"). Such dialogs are switched off, and
# one that still appears is closed and logged.
"$adb" shell settings put global hide_error_dialogs 1 || true
"$adb" shell am broadcast -a android.intent.action.CLOSE_SYSTEM_DIALOGS >/dev/null 2>&1 || true

# Which window had the focus while the tests ran (changes only), so that a
# system dialog lying over the app can be told from a defect in the app.
focus_log="${TMPDIR:-/tmp}/vw-focus.log"
(
  set +e
  last=""; count=0
  while :; do
    now="$("$adb" shell dumpsys window </dev/null 2>/dev/null | tr -d '\r' \
      | grep -E 'mCurrentFocus=|mFocusedApp=' | sed 's/^ *//' | sort -u | tr '\n' ' ')"
    count=$((count + 1))
    if [ -n "$now" ] && [ "$now" != "$last" ]; then echo "$(date +%T) sample $count: $now"; last="$now"; fi
    case "$now" in *"Application Not Responding"*|*"Application Error"*)
      "$adb" shell am broadcast -a android.intent.action.CLOSE_SYSTEM_DIALOGS </dev/null >/dev/null 2>&1
      echo "$(date +%T) sample $count: closed a system error dialog" ;;
    esac
    echo "$count" >"$focus_log.count"
    sleep 1
  done
) >"$focus_log" 2>/dev/null &
focus_pid=$!
trap 'kill "$focus_pid" 2>/dev/null || true; "$adb" emu kill >/dev/null 2>&1 || true; kill "$emulator_pid" 2>/dev/null || true' EXIT

step "instrumented tests: $classes"
status=0
bash apps/gradlew -p apps :android:connectedDebugAndroidTest -PvwEmulatorAbi=x86_64 \
  "-Pandroid.testInstrumentationRunnerArguments.class=$classes" \
  -x :desktop:packageConnectionRuntime --console=plain --no-daemon --no-parallel --max-workers="${VW_GRADLE_WORKERS:-2}" || status=$?
kill "$focus_pid" 2>/dev/null || true
step "result summary"
"$adb" shell wm size || true
"$adb" shell wm density || true
if [ "$status" -ne 0 ]; then
  # The app's own warnings and errors, to tell an emulator limit from a defect.
  "$adb" logcat -d -v brief '*:W' | grep -iE "visualworkbench|VW_|vw_core|AndroidRuntime" | tail -n 40 || true
  # For each failed screen check: the canvas's own account from that test's
  # start to the failure, and the platform's lines about stalled drawing.
  echo "--- canvas lifecycle of each failed on-screen test ---"
  # A test that fails in its setup wait has no present_failed line; the test
  # runner's own "failed:" line closes its block instead.
  "$adb" logcat -d -v time -s VW_CANVAS_LIFE:I TestRunner:E | awk '/ test_begin /{n=0} {b[n++]=$0} / present_failed |TestRunner.*failed: /{for(i=0;i<n;i++)print b[i]; n=0}' | tail -n 400 || true
  echo "--- platform lines about drawing ---"
  "$adb" logcat -d -v time '*:I' | grep -E "dequeueBuffer failed|SurfaceSyncGroup|BufferPool|LowLatencyCanvas|Davey|Skipped [0-9]+ frames|queue stalled" | tail -n 60 || true
  echo "--- focused window during the tests (changes only) ---"
  tail -n 60 "$focus_log" || true
  echo "focus samples taken: $(cat "$focus_log.count" 2>/dev/null || echo 0)"
fi
"$PYTHON" - <<'PY' || true
import glob, xml.etree.ElementTree as ET
total = failed = 0
for path in glob.glob("apps/android/build/outputs/androidTest-results/connected/**/*.xml", recursive=True):
    for case in ET.parse(path).getroot().iter("testcase"):
        total += 1
        problem = case.find("failure") if case.find("failure") is not None else case.find("error")
        if problem is not None:
            failed += 1
            first = (problem.text or problem.get("message") or "").strip().splitlines()[:1]
            print(f"FAILED {case.get('classname')}.{case.get('name')}: {first[0][:300] if first else ''}")
print(f"on-screen tests: {total} run, {failed} failed")
PY
exit "$status"
