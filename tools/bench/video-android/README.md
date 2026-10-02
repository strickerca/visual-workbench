# S23 generated video receiver

Diagnostic-only `:video-bench` module, package
`com.visualworkbench.videobench`. See the companion
[`video-pc` README](../video-pc/README.md) for methods, commands and limitations.

The activity accepts bounded generated HEVC fixtures in an app-owned UUID
directory, or JPEG tiles from a loopback socket forwarded by the project runner.
It requires a live SurfaceView and does work on its own worker thread. Lifecycle
cancellation fails the run explicitly. No private user media or unrestricted
network destination is accepted. Results contain numeric timings, codec names,
requested controls, frame counts and safe failure phases; exception messages and
device identifiers are excluded.

Use `run_android.ps1` for HEVC and `run_tiles.ps1` for recorded dirty-tile delivery.
Both select the authorized SM-S918U explicitly and retain per-attempt text
receipts. Installation uses the current source-bound diagnostic APK. The runners
check foreground state and bounded readiness; they do not wait indefinitely on a
launcher or reset the shared adb server.
