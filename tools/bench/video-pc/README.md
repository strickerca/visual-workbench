# T0.07 generated video benchmark

This diagnostic captures only its own generated native Windows window. It does
not accept arbitrary window handles or capture the desktop. Run the gated builds
before HIL; the runners reject binaries whose recorded source bindings are stale.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 build-video-pc
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 build-video-android
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test video-pc
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test video-android -VideoRunId <reported-id>
powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 hil-test video-tiles -VideoRunId <reported-id>
```

Use `run_pc.ps1 -Profile smoke` for diagnosis before a full run. Its native windows
show generated content without activating or injecting input. Native dimensions
are checked, DPI awareness is Per-Monitor V2, WGC's minimum update interval is
explicitly 33.3333 ms, and DirtyRegions uses ReportOnly. Cursor capture is off.
Actual capture timestamp intervals and reported dirty rectangles are measured;
the requested interval is not a claim of sustained 30 fps.

Each profile runs in a fresh bounded native process, with an aggregate receipt
saved after each worker. A multi-profile full run faulted in
`IntelControlLib.dll_unloaded` (Windows Application error 1000, `c0000005`) after
successful HEVC phases. Process isolation limits cross-profile damage; it does
not establish that the driver lifetime bug is fixed or safe for an integrated
long-running streaming service. A worker must both write its completed report
and exit successfully to pass. The optional native phase `all` retains the
multi-profile path for explicit future diagnosis.

The HEVC path requires a registered hardware MFT, D3D11 awareness, a DXGI device
manager and accepted low-latency, zero-B-frame and capped-bitrate controls. GPU
video processing converts BGRA to fresh NV12 textures. No CPU readback supplies
the encoder. One frame is outstanding; output must carry that frame's timestamp.
Submit-to-output time excludes GPU conversion submission, capture and network
delivery. Accepted controls and no observed output reordering do not establish a
complete bitstream conformance audit. The benchmark fails rather than silently
using a software encoder or an unbounded queue.

Portrait 1440 x 3088 and 3840 x 2160 each use ten excluded warmups and 120 measured
frames (12 in smoke mode). Annex-B bytes and a bounded sample index are written
outside the repository for the companion Android diagnostic. Android records
hardware decoder selection, requested KEY_LOW_LATENCY, advertised vendor integer
parameters, maximum advertised operating rate, queue-to-output time, and Surface
render callback census/timing. Callback delivery uses the app's monotonic clock;
the codec's supplied render timestamp is recorded separately and rejected as a
latency measurement if it precedes queueing. Requested configuration is
distinguished from observed behavior. No cross-device clock synchronization or
photon timing is claimed; callback dispatch can itself be delayed and batched.

The raster path reads WGC pixels back to the CPU and measures that readback
separately. The initial run measured WGC-reported rectangles directly and observed
full-window regions despite partial source changes. The revised path compares
consecutive RGB frames, coalesces tight changed row spans, and measures that CPU
difference step separately. A reconstruction test verifies coverage of every
changed pixel, including a bounded full-frame fallback for fragmented changes.
It encodes derived regions as JPEG quality 85, recording both WGC-reported and
encoded area, encode/copy/serialization time and bytes. Unchanged captures are
counted and excluded from dirty-frame measurements; attempts are bounded. The
requested 10/25 percent regions must be compared with the actual observations.
PNG fast/Sub and QOI encode the full 4K frame, with two excluded warmups and ten
measurements; exact decoded RGB equality is checked outside the encode timer.
All percentiles use nearest rank.

`serve_tiles.py` reads generated `.vwt` recordings and serves only loopback. The
S23 runner creates an owned adb reverse mapping with no-rebind, then transfers
recorded JPEG frames and waits for a sequence and SHA-256 acknowledgment after
phone decoding, canvas reconstruction and Surface posting. Full runs use ten
warmups and 120 measured frames; smoke uses two and ten. One additional initial
full frame is sent and excluded so static content is initialized. Only recorded
dirty frames cycle.
This measures real USB delivery and phone work, but excludes concurrent PC capture
and encoding. Surface posting does not prove display presentation. Report these
numbers separately from PC raster cost and product STREAM-001 acceptance.

Media is stored in a fresh `%TEMP%/VW-video-<id>` directory bound to an ignored
`.local/video-run-<id>.json` receipt. Inspect the generated preview and finish
phone use, then run:

```powershell
python tools/bench/video-pc/dispose_media.py <id> --inspected
```

The disposer checks exact task ownership, retains text hashes/counts, and removes
only owned PNG, HEVC and tile-recording files. Keep all verification media out of
Git. Failed run receipts remain evidence of failure and must not be overwritten.
Phone runners select SM-S918U only, remove their own files/mappings, and stop only
the diagnostic package. They neither reset adb nor operate on a reserved peer.

Primary API references:

- [WGC minimum interval](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.minupdateinterval?view=winrt-26100)
- [WGC dirty regions](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.dirtyregionmode?view=winrt-26100)
- [Media Foundation HEVC encoder](https://learn.microsoft.com/en-us/windows/win32/medfound/h-265---hevc-video-encoder)
- [Low latency control](https://learn.microsoft.com/en-us/windows/win32/medfound/codecapi-avlowlatencymode)
- [Android MediaFormat](https://developer.android.com/reference/android/media/MediaFormat)
- [Android MediaCodec](https://developer.android.com/reference/android/media/MediaCodec)
- [Android render callback timing](https://developer.android.com/reference/android/media/MediaCodec.OnFrameRenderedListener)

Qualcomm parameter names are queried against the selected codec's runtime vendor
parameter descriptors. Their presence/request is reported; undocumented vendor
semantics are not represented as independently verified.
