# Android editor integration

The app uses the common `WorkbenchCore`/`WorkbenchProject` façade. JNI bindings,
the prebuilt Rust library and its ELF alignment check are supplied by `:shared`.
The root `build.ps1` lane owns compilation and hardware runs. On 2026-10-03,
the isolated, exact-native-bound app passed all 65 instrumentation cases on the
physical IN2019 (Android 11). Owned packages and private run files were removed.
This covers synthetic input/provider/lifecycle/export fixtures; real camera,
pairing, S23/S Pen latency and owner acceptance remain open.

## Implemented paths

- App-private project creation, file-backed PNG/JPEG/WebP import and durable reopen.
  The original encoded asset is passed unchanged to the core. Images exceeding
  50 million pixels receive the specified refusal; no downsampled replacement is
  silently made. Existing damaged projects remain listed and preserved.
- Generic stylus MotionEvents, historical/coalesced samples, raw pressure
  diagnostics, normalized stroke pressure, optional tilt/orientation, hover,
  eraser tool identity and button capability reporting. Samsung actions are an
  optional disabled hook. Finger navigation is the default; finger drawing is a
  saved setting. Cancellation and palm transitions discard unfinished edits.
- Core camera mapping in f64, pan/pinch/rotation with cardinal snapping,
  two/three-finger undo/redo, long-press context controls, and a movable mirrored
  rail. Screen input is converted through the core, with local origins before
  paths enter Android float rendering.
- Pen/marker geometry and disposable prediction from the native ink worker.
  Highlighter uses a multiply layer. Queues are bounded; overflow cancels the
  unfinished stroke visibly. The opaque front scene includes its backdrop so
  multiply compositing is correct across SurfaceControls. Rendering lifecycle
  generations retire before replacement; published paths are immutable.
- Editable line, arrow, rectangle, ellipse and bundled-font text; existing text
  retains its object ID. Move/resize uses provisional affine transforms and one
  transaction at pen-up. Erasing intersects actual path geometry with the swept
  eraser, then emits a single batch of whole-object deletion operations. It never
  persists an unmaterialized vector-eraser stroke.
- Property changes and undo/redo persist through the core. Per-brush monotonic
  Bézier pressure curves and stabilization persist locally, with an actual core
  outline preview. Existing-object style edits update native stroke width too.
- Full-resolution PNG/JPEG/WebP export through native preflight and encoding;
  reducing a 16-bit source requires explicit consent. Untagged display color requires explicit sRGB consent. The
  original asset remains unchanged by display conversion and export.
- Project, canvas, settings, raw diagnostics and the integrated T1.10 pairing
  screen. Physical pairing and reconnect acceptance are still pending.

## Validation supplied for the root runner

`StartupInstrumentedTest` invokes the packaged native core. The new test classes
are `GestureInstrumentedTest`, `EditorCoreInstrumentedTest`,
`EditorSurfaceInstrumentedTest`, `EditorPreferencesInstrumentedTest` and
`TouchTargetsInstrumentedTest`.

They cover generic MotionEvent mapping, gesture policy, native typed edits and
reopen, highlighter blend, actual stroke-width editing and undo, pressure/curve
and stabilization geometry changes, disposable prediction, precise erase hits,
real app SurfaceView tool events, cancellation, activity recreation, preference
roundtrip, declared palette contrast and accessible project-action hit bounds.
All input is synthetic; it cannot prove S Pen hardware behavior or comfort.

`RasterTransfersInstrumentedTest` adds real native file import/export, source
preservation and scratch cleanup, cancellation of a partial export, and a native
handle returned during import cancellation. `EditorOwnershipInstrumentedTest`
covers disposal during project creation. These source additions have not been
compiled or executed in this worktree; the parent owns the serialized runner.

## File-backed image transfers

`RasterTransfers` copies provider streams in 64 KiB chunks into a unique private
cache workspace. The native `WorkbenchStreamingCore`/`WorkbenchStreamingProject`
interfaces perform decode, orientation, ICC handling and PNG encoding. Android
does not duplicate those algorithms or allocate a whole imported/exported byte
array. PNG8/16 decoding uses disk scratch and bounded strips; admitted baseline
JPEG decoding uses a compact RGB/luma buffer followed by scratch rows. Smaller
progressive JPEG and WebP files use the guarded buffered fallback. JPEG/WebP
export uses the additive `WorkbenchFileTransfers` bounded buffered path; PNG
export retains strip streaming. Preflight reports the native reservation and
whether marked geometry/result assets still require rendering-time validation.

The native limits are 50 million pixels, 64 MiB encoded input, 256 MiB accounted
working memory, 400,000,000 bytes of source scratch, and 512 MiB encoded output.
The scratch and encoded output may coexist, so these limits do not promise that
an export fits on a nearly full disk. Native accounted working memory is not
total application RSS; the current display bitmap and project state also exist.
The original encoded bytes are immutable. No import or export silently resizes
the image, reduces its depth, substitutes a profile, or flattens transparency.

The UI reports the current transfer stage and offers cancellation. Shared Kotlin
waits for the native producer to settle before private temporary files are
removed. A created project that was atomically published before cancellation is
kept and its returned handle is closed. Native exports publish a completed
private file without replacing any existing path; Android opens the new
CreateDocument URI only after that receipt arrives, then copies the file in
chunks. On cancellation or failure it removes its new destination. Providers
that refuse deletion produce an explicit partial-file warning. The application
cannot promise atomic publication or deletion inside an arbitrary third-party
document provider.

Cleanup checks exact workspace ownership and removes only direct files. Normal
completion, cancellation and failures have cleanup paths. Process-kill orphan
discovery is not yet implemented. Export is available even when the full-buffer
display adapter refuses a large image. The receipt binds the accepted host
sequence and the visible project hash, including queued offline edits.

## T1.11 Android intake and handoff candidate

The manifest accepts one `image/*` share through `ACTION_SEND`. Photos uses
AndroidX's photo picker with its platform fallback; Files uses `OpenDocument`.
Transient URI access is consumed by a bounded original-file copy while the
process holds the grant. No long-lived URI permission is retained. A shared
image arriving during startup waits for initialization instead of being dropped;
only one incoming import waits at a time. Rust identifies the encoded format,
verifies dimensions, preserves original bytes and creates the content-addressed
asset. Unsupported picker content is refused explicitly.

Take photo delegates to a camera activity with a full-resolution `EXTRA_OUTPUT`
URI, ClipData and temporary read/write grants. No camera permission, thumbnail
replacement or in-app camera is used. The capture token survives Activity
recreation. Captures live in the durable private `files/camera-inbox`, with up to
64 pending capture slots. A failed import or final ViewModel disposal retains
the full-resolution original for Retry, Save original, or an explicit confirmed
Discard action. Completed captures never expire automatically. Successful
durable import and explicit transfer cancellation remove the capture only after
the importer has settled. Actual camera app behavior
and process-death result delivery still require physical verification.

The export sheet offers clean/marked, full image/current view bounds, PNG8/16,
JPEG and lossless/lossy WebP. The view crop is an image-aligned integer rectangle
clipped to the original, including when the camera view is rotated. Quality,
JPEG white matte, 8-bit reduction and sRGB conversion are explicit choices.
Native preflight must succeed before saving or sharing. View changes invalidate
a checked view crop; project edits invalidate preflight and EXPORT READY.
An immutable picker ticket binds project, document, host sequence, visible hash,
format and exact export options across activity recreation. A stale result is
refused and its newly created destination is removed. JPEG matte does not carry
into PNG or WebP when the chosen format changes.
Completed receipts show revision and exact output dimensions, and bind the
visible state hash so offline edits cannot reuse an old ready status.

Share PNG provides one PNG with no text attachment and a temporary read grant.
The non-exported FileProvider exposes only `files/camera-inbox` and
`cache/media-handoff/share`; neither projects nor native scratch are exposed.
Successfully handed-off PNGs remain readable for 24 hours and are pruned on a
later app start. Canceled/unlaunched handoffs are removed immediately. Cleanup
checks exact token, canonical containment and direct-file ownership, rejects
symlinks, and protects durable camera originals and a restored active capture.
Cleanup failures are reported without crashing the editor. A recipient that copies the
image has its own copy; the app does not claim control of recipient retention.

New source regressions cover intent/grant fields, actual provider containment,
expiry/active capture preservation, symlink rejection, rotated crop bounds,
state-hash readiness invalidation, typed preflight failure cleanup, share
handoff cancellation and successful disposable-file ownership. Existing touch
target assertions include Photos, Files and Camera. These synthetic cases passed
in the central 65-case IN2019 run. No personal images, actual camera capture,
outgoing third-party share or verification screenshot was used in that run.
Provider opens/queries receive a CancellationSignal and are isolated behind a
bounded two-worker/eight-queue pool with a 15-second response limit. Cancellation
signals use their own bounded worker. Late descriptor returns are closed.
Provider reads/writes also run in that bounded pool, using nonblocking
descriptors and 100 ms readiness polling. Each call owns one duplicate
descriptor and a private buffer capped at 64 KiB, including when a cloud/FUSE
provider blocks inside an apparently ready syscall. Queued cancellation closes
the unclaimed duplicate; a running syscall retains its duplicate until return,
then closes it without touching caller buffers or app scratch. Newly created destination
cleanup has a two-second limit and reports possible leftovers. A hostile remote
provider can ignore cancellation; it cannot indefinitely retain the caller or
spawn unbounded local workers, but its own process and external side effects are
outside this app's control. Tests retain idle pipe endpoints while canceling the
real transfer and assert that owned descriptors/scratch are closed. Separate
proxy-file fixtures block both read and write callbacks, assert that cancellation
settles before the provider returns, and verify late descriptor/buffer ownership.

Third-party destinations, airplane-mode sharing, every-format metadata, video
asset sync and all-device status observations remain parent/owner acceptance
work. This candidate does not satisfy those physical or transport clauses.

The text golden compares Android path rendering against native PNG export at
1:1 in memory. Its `VW_TEXT_GOLDEN` receipt records exact changed pixels,
interior mismatches, mean error and maximum error. An edge-tolerant software test
pass is **not** bit-exact text acceptance. No screenshots or golden image files
are written. `VW_CONTRAST` records ratios for each declared text pair;
`VW_TOUCH_TARGET` records measured project-action bounds in both themes.

## Open acceptance clauses

- The 256 MiB native display path still refuses some images below 50 megapixels.
  The new bounded PNG transfer path requires actual compiler, strip-parity,
  large-image and device validation. Progressive JPEG, unusual JPEG sampling,
  WebP, very complex marks and large result-layer images may exceed their guarded
  decoder/compositor budget. These are open T1.05/T1.08 clauses, not accepted
  reductions of the requirement. Original bytes and saved edits remain intact.
- Android Skia and native tiny-skia may differ at antialiased text edges. Exact
  parity remains open unless the actual comparison reports zero changed pixels.
- A full control-by-control 48 dp audit, all-state contrast, landscape and
  left-handed usability review, system font scaling, physical pen traces,
  compositor presentation timing and owner usability acceptance still need the
  serialized runner and/or owner. Draw callbacks never substitute for PERF-001.
- Shapes from future PDF/result/adjustment modules are explicitly refused when
  the editor lacks their display adapter. Their model data and original assets
  are preserved. The T1.10 source increment below wires native live sync; its
  cross-device and physical acceptance remains unverified.
- The current fonts intentionally refuse missing glyphs; there is no host-font
  fallback. Failed text remains in the text editor for correction.
- T2.01 integration is specified in [SELECTION_INTEGRATION.md](SELECTION_INTEGRATION.md).
  Selection tools, mask history/export and stroke-splitting erasure are not yet
  exposed as functioning application controls.

## Dependency changes

The app declares the already used exact pins `graphics-core:1.0.4`,
`input-motionprediction:1.0.0`, `lifecycle-viewmodel-ktx:2.9.4`, and
`kotlinx-coroutines-android:1.9.0`. The root integrator updates dependency locks
and runs the central license gate before compilation. No runtime downloads,
credentials, paid calls, extra native libraries or phone operations occur here.

## T1.10 Android session integration — source candidate

The pairing screen now uses CameraX image analysis and ZXing core to decode the
bounded shared `vw-pair:v1:` QR text envelope. Native code alone interprets the
credential, checks its expiry and certificate binding, and performs the pairing
handshake. The code fallback requires an explicit complete fingerprint
comparison before confirmation. Pending QR bytes are cleared on background,
screen exit, expiry and failure; codes are never saved. The pairing screen uses
FLAG_SECURE, and camera frames are analysis-only (one latest frame, <=2 MP,
one worker, no saved files or logging). Camera denial/unavailability retains
the code path. Declaring CAMERA also required the existing full-resolution
external-camera flow to request permission before launch.

One lazily acquired session service uses the repository's existing installation
DeviceId and the shared Android Keystore trust adapter. It is closed only after
project links and collectors. Paired computers can be selected and revoked.
The NsdManager adapter browses `_vworkbench._udp`, limits callbacks/resolve queues
and entries to 64, resolves one service at a time with a five-second deadline,
and gives TXT/address hints to the native strict TTL catalog. Discovery never
authenticates a device. No absent listener is advertised. Each configured
numeric route is distinct: native reconnect uses tether, Wi-Fi, then optional
ADB order. Phone hosting offers actual local-address choices; receiving a
computer project uses authenticated native bootstrap into a new app-private
UUID directory before adoption. Host identity is never reassigned implicitly.

The editor collects project changes through one conflated refresh queue, with
project epochs and increasing event sequences fencing asynchronous snapshots.
A document and its own native revision are published together, including
optimistic states with equal host sequence but different hashes. Remote edits
invalidate export readiness. Collectors and the link close before their project
handle. Real native stroke handles broadcast real accepted sample batches only;
motion prediction remains local. Handle/slider previews carry one gesture ID
into the final transaction. New shapes retain identity/layer/kind/time while
updating bounded document-space geometry, preserving normal outline widths;
their exact final geometry and identity are committed. Cancellation retires
previews and committed closure follows the durable write. Native minor-2
Open/Close generation fencing owns delayed/replayed packet admission.

Native peer geometry is polled through one bounded query worker and drawn with
the same app path adapter. Status shows SYNCED, SYNCING, RECONNECTING, OFFLINE
pending/blocked counts and actual echo RTT samples when present. Peer viewport
outlines are separate from the camera. Only explicit Match/Follow moves it;
manual pan/zoom/fit stops following. A USB-tethering shortcut is available.

Eleven new synthetic instrumentation test sources cover stale refresh completion
and same-base optimistic ordering, installation identity and late service
ownership, QR clearing after late inspection, explicit confirmation, bounded
numeric routes, controller remote refresh without camera motion, real-only
stroke streaming/commit order, background cancellation and link-before-project
close, explicit viewport adoption, and new-shape final geometry/identity parity.
They have **not been compiled or run by this author**. Camera/permission denial,
actual DNS-SD, Keystore persistence/revocation, real pairing/transport/reconnect,
forced conflicts, original uploads, no-developer-mode/WAN-unplugged scenarios,
S23/S Pen timing and owner acceptance still require parent validation. Desktop
route/firewall/ADB management is outside this Android adapter. A native/app
status or source regression does not establish those hardware clauses.

Dependency provenance checked 2026-10-02: CameraX `camera-core`, `camera-camera2`,
`camera-lifecycle`, `camera-view` **1.6.2**, stable release dated 2026-08-26
([official releases](https://developer.android.com/jetpack/androidx/releases/camera)),
and ZXing `core` **3.5.4**
([official release](https://github.com/zxing/zxing/releases/tag/zxing-3.5.4),
[Apache-2.0 license](https://github.com/zxing/zxing/blob/zxing-3.5.4/LICENSE)).
CameraX source uses Apache-2.0; its camera-core build also declares bundled
libyuv BSD licensing ([official source](https://android.googlesource.com/platform/frameworks/support/%2B/4aa409d104231ea63ba917270cd1b2c3dd747353/camera/camera-core/build.gradle)).
The root's central license gate must resolve and inspect all artifact/transitive
licenses and bundled native libraries, including APK ELF/ZIP alignment, before
device execution.
API references: [CameraX analysis lifecycle](https://developer.android.com/media/camera/camerax/analyze),
[NsdManager discovery](https://developer.android.com/develop/connectivity/wifi/use-nsd),
[local-network permission policy](https://developer.android.com/privacy-and-security/local-network-permission).
At target SDK 36, INTERNET provides local-network access under the documented
compatibility policy. A target-37 upgrade must add its required local-network
permission or system-mediated picker; this slice does not silently raise target SDK.
No runtime downloads, actual camera/media reads, outgoing shares, phone commands,
builds, tests or Git publication were performed here.
