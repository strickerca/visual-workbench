# Pen probe and replay

This is a separate diagnostic app, `com.visualworkbench.penprobe`. It does not
replace the Workbench starter. Build from the repository root:

    powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 build-pen-probe
    powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test pen

The second command builds, installs and tests on the one connected physical
device. It runs a synthetic software check, collects private JSON metrics under
`.local/pen-hil-<run>/`, and stops only the probe and its instrumentation package.
It never counts synthetic events as physical stylus evidence. A lost foreground
state makes a shared-phone run inconclusive. It does not reset adb or stop the
other app. The installed **Workbench Pen Probe** remains available to open.

## Record the owner traces

Use the S23 Ultra and S Pen in portrait. Keep the phone unlocked and connected.
Open Workbench Pen Probe. Choose each trace from the list, tap **Record**, draw
in the white canvas, then tap **Save trace**. The next category is selected after
saving. Use only test shapes and the word **ink** because the repository is public.
There are 24 required recordings:

1. Five straight lines: two slow, three fast.
2. Three circles, then three short handwritten instances of “ink”.
3. Three quick flicks, then two hover-only passes without touching the screen.
4. Two strokes with the palm resting on the screen.
5. Two strokes with a barrel-button press during contact.
6. Two strokes with the pen leaning clearly toward the right edge, then two
   leaning toward the bottom edge. Keep the phone in portrait for all four.

Also record `air-command-on` and `air-command-off` with that setting enabled and
disabled by the owner. For each, press the barrel button during hover and during
contact. Note whether Air Command opened and whether the probe still received
the event. If it backgrounds the app, the saved trace is marked
`interrupted_background`; it cannot satisfy complete-trace acceptance.

Enable the probe in **Settings → Advanced features → S Pen → Air actions** if
needed. In `air-actions`, try single/double click, left/right/up/down swipe, and
clockwise/counterclockwise circles. F1–F8 in the probe show which mapping arrived.
Manual enabling may be needed: the optional Samsung `enable_key` is not supplied.
Record eraser behavior in `eraser` if an eraser tool is available; otherwise say
it was not available. Do not infer absent axes or capabilities from constant zero.

Saving uses app-private storage and a unique filename. No network or broad storage
permission is requested. At 30,000 pointer samples/events a recording stops with
an explicit limit marker. Keep each acceptance trace below 120 seconds for replay.
The JSON retains every pointer and history frame, axes 0–63, action, button state,
action button, flags/cancellation, source, numeric device ID, key events and
declared device axis ranges. Numeric input IDs are runtime IDs, not serial numbers.
Device names/descriptors, serials, account data, network identifiers and typed key
characters are never collected. Axes 0/1/2/3/8/24/25 mean x/y/pressure/size/
orientation/distance/tilt. Pressure is Android's calibrated value; hardware ADC
counts are not exposed by MotionEvent.

## Pull, inspect and replay

    powershell -NoProfile -ExecutionPolicy Bypass -File tools/pen-trace/collect-traces.ps1
    python tools/pen-trace/trace_tool.py summarize fixtures-private/<collected-directory>

Collection keeps originals on the phone and writes only into ignored private
storage. Inspect the trace content and metadata before copying reviewed JSON
files into `fixtures/traces/`. The validator rejects unknown fields, duplicate
keys, non-finite axes, invalid history/pointers and inconsistent timestamps.

    python tools/pen-trace/trace_tool.py acceptance fixtures/traces
    powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test pen-owner -TimeoutSeconds 1200

Owner mode requires the full 24 named S23 Ultra recordings before any device
action. The build embeds the reviewed traces in test assets. It never rescales
coordinates; the recorded canvas must fit on the replay device. Replay uses
`UiAutomation.injectInputEvent` and records the resulting callbacks independently.
It checks exact pointer-sample counts, all 64 axes, pointer/tool IDs, actions,
button states, cancellation, source, modifier state and Air Action key delivery.
Dispatch can rebatch motion histories, so comparison flattens batches into samples.

Original timestamps belong to an earlier uptime epoch. Replay shifts the entire
trace to a new start time, rounds to the public `MotionEvent.obtain` millisecond
time base, and compares received timestamps after removing that single shift.
The report includes maximum and nearest-rank p95 error (target ≤1 ms), and separate
wall-clock injection lateness. The timestamp check does not prove real-time
scheduling or physical input latency. Android can alter injected device IDs and
system flags; the original trace retains these, and only the cancellation flag
is asserted as a semantic flag. Numeric device identity is not forged on replay.

Known platform limit: `getActionButton()` has no public setter. Traces containing
a nonzero action button fail explicitly; they are not silently normalized.
Non-Air-Action keys are also rejected so a replay cannot invoke Home, Power or
another app. Classification is recorded but not synthesized on Android 11.
Physical Samsung replay may expose further OS resampling or routing differences.

## Measurement methods

The summary computes contact and hover rates separately from positive sample
intervals within each pen stream, including history. It does not span separate
strokes. Observed unique pressure values and their smallest step are sample
statistics, not proof of the advertised number of hardware pressure levels.
Tilt/orientation ranges preserve their signs; the owner labels the four known
lean directions so the later Windows conversion can be calibrated.

The renderer uses `androidx.graphics:graphics-core:1.0.4` and a front-buffered
SurfaceView. It draws the latest contact point as a pressure-sized dot. Recorded
timings compare Android uptime event time with the front-buffer draw callback and
the next Choreographer callback. These are CPU scheduling/render-submission
estimates, not panel presentation or input-to-photon latency. Choreographer's
frame timestamp and callback time are retained separately. API 34+ records the
public nanosecond event/history timestamps; older Android records milliseconds
multiplied by one million and labels that resolution. Renderer/recorder overhead,
USB power, refresh rate and other active apps affect measurements.

## Ten-minute thermal session

After the owner confirms readiness, open the probe and draw casually in its
canvas for ten minutes. The operator runs:

    powershell -NoProfile -ExecutionPolicy Bypass -File tools/pen-trace/thermal.ps1 -OwnerReady -OutFile fixtures-private/thermal-new.json

The logger records battery temperature and power state at 0, 30, …, 600 seconds,
with actual elapsed times. It checks foreground at each sample, reports progress
at most 15 seconds apart and preserves an incomplete result on failure. It never
claims drawing continuity from temperature alone: the owner must confirm that.
No thermal session is started automatically by HIL. Short diagnostic durations
are explicitly not ten-minute baseline evidence.

## Official references

- [Samsung declarative Air Actions](https://developer.samsung.com/galaxy-spen-remote/air-actions.html)
- [Android MotionEvent API and time precision](https://developer.android.com/reference/android/view/MotionEvent)
- [Graphics stable releases](https://developer.android.com/jetpack/androidx/releases/graphics)
- [Front-buffer renderer callbacks](https://developer.android.com/reference/kotlin/androidx/graphics/lowlatency/CanvasFrontBufferedRenderer.Callback)

These APIs do not establish what the owner's S Pen actually delivers. That comes
from the physical recordings and the capability table in `docs/evidence/T0.04.md`.
