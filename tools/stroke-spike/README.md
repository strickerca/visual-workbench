# Stroke engine comparison

The provisional shared Rust engine is in `core/crates/vw-ink`. This tool keeps
the native fixture/CPU benchmark runner, a bounded primitive-only JNI bridge,
and an Android comparison of core-generated ink with Jetpack Ink **1.0.0**.
The final D6 selection still requires the owner's blind S Pen comparison and
target-device latency; software runs on IN2019 do not replace those checks.

Build and test from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 build-stroke-core
cargo +1.99.0 test --locked -p vw-ink -p vw-stroke-jni
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 build-stroke
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test stroke
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test rust vw-ink
```

The HIL runner uses the explicit authorized device model recorded by the shared
device selector. It checks exact source/APK/native build bindings, compares all
16 synthetic native hashes, benchmarks 12,800 optimized append calls, and runs
three real Android instrumentation tests. Reports require a fresh random run ID.
Only this tool's unique temporary binary and two app packages are cleaned up.
No verification image files are created: the wet/dry pixel comparison happens
in memory, and its buffers are recycled. Text receipts remain in ignored `.local`.

The native registry exposes monotonic IDs, never pointers. Eight live handles,
2,048 samples per stroke, 64 started demo strokes between clears and bounded
metrics prevent unbounded diagnostic accumulation. Predictions use disposable
clones and never enter the committed geometry. Multi-pointer/cancel input and
activity loss cancel the provisional stroke. Queued front-buffer work coalesces;
each callback records the timestamp of the immutable geometry it actually drew.
Dry commits hold immutable snapshots, and clear/surface retirement closes the
render generation. Jetpack clears also discard finished strokes arriving later.

The two drawing surfaces use the same nominal color and width and receive the
same physical or synthetic MotionEvents. Demo 1/2 assignment is randomized and
stored privately. The preference dialog records handwriting, circles and fast
lines, with optional comments, only after a person explicitly chooses an answer.
It does not infer a preference from software tests. App backup is disabled.

Callback timing uses Android's monotonic uptime clock with 1 ms resolution.
Rust front/multi-buffer callbacks and Jetpack authoring/finished callbacks are
different pipeline stages. Their numbers are **not comparable photon latency**,
do not include display presentation, and cannot settle D6. The native CPU append
benchmark separately excludes JNI, prediction cloning, rendering and display.

Pinned API references: [Ink release notes](https://developer.android.com/jetpack/androidx/releases/ink),
[InProgressStrokesView](https://developer.android.com/reference/androidx/ink/authoring/InProgressStrokesView),
[MotionEventPredictor](https://developer.android.com/reference/androidx/input/motionprediction/MotionEventPredictor),
[input release notes](https://developer.android.com/jetpack/androidx/releases/input).

The Windows Google Ink diagnostic is `try-google-ink.ps1`. It expects an isolated
checkout and Bazel executable prepared from the source/release links in T0.10's
evidence. Record the absolute task-owned `%TEMP%/vw-google-ink-<32 hex>` directory
(containing `source` and `bazel.exe`) in ignored
`.local/t010-google-ink-location.txt`. The runner verifies the exact upstream
commit, clean tracked source and executable digest before running. Optional
`-MsvcCxx20 -ShortOutputBase -MsvcMathDefines` flags reproduce the final diagnosed
configuration. It uses a bounded two-worker Windows job and records every result;
no system compiler policy or upstream tracked source is modified.
