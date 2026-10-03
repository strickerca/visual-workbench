# Desktop editor implementation

Central checkpoint (2026-10-03): the complete desktop and shared JVM suites passed
again in `build.ps1 test-all`, including the native cache-anchor regressions.
The packaged Windows startup runner passed its 24 parser/binding, ten bounded
child-capture and Windows lease fixtures. The actual packaged application drew
a frame at density 1.75/window DPI 168 and completed ordinary shutdown; see
`docs/evidence/T1.09.md` for exact receipts and timing limits. Synthetic tests do not
establish owner visual, input, clipboard/drag or paired-device acceptance.

The window uses the generated shared core and host bindings. PMv2 is established
before the first AWT window; native per-window DPI and Compose density are shown
in the status bar. A first-composed-frame startup measurement is emitted without
identifiers. Windowed bounds and all three placement modes are saved, and F11
returns to the previous placement. Off-screen saved positions return to center.

Image import, project open, typed render lists, core text outlines, canonical ink,
mouse move/resize, color/width changes, deletion, undo/redo, document-pixel nudges,
file export and the host's PNG/DIB clipboard formats use native APIs in source.
One pointer gesture or completed width slider edit creates one transaction. The
command queue is bounded and reliable mutations serialize; relative keyboard
nudges resolve against the latest accepted state. Full-size backgrounds require
explicit untagged-sRGB consent, retain the source original, and have a 256 MiB
image working budget. Export defaults to the source depth and exposes explicit
format, depth, matte and color choices in the export sheet.
Result/adjustment composition in the interactive canvas is explicitly unavailable;
such a project is preserved and refused for partial display.

Canvas gestures capture the project attachment, host sequence and state hash.
The shared facade passes those preconditions to the native worker's atomic edit
check; a concurrent peer edit refuses the stale gesture. Older or duplicate
refresh responses cannot replace a newer document or clear its gesture preview.
Text drafts remain open after a rejected edit. Prepared paths and their document
snapshot publish together; a new revision or failed preparation hides the old
frame. Screen-constant outlines are stroked after the object's full transform,
and marker circle/box passes composite separately like the canonical rasterizer.

Ordinary shortcuts apply only while the canvas has keyboard focus. Buttons,
sliders and text dialogs retain their own Tab, Enter and arrow handling. Click
the canvas to focus it; Shift+Tab leaves it for standard focus traversal. Modified
application shortcuts and F11/F12 remain available outside modal dialogs.

The camera is local. Receipt refreshes never replace it. Follow and match require
an actual peer viewport, and following/outlines are off by default. Mouse drag,
wheel/two-finger scroll, Ctrl+wheel zoom and touch-classified one-finger pan are
wired. AWT on Windows does not expose general touchscreen multi-touch. Native
trackpad magnify delivery, high-DPI crispness, movement between monitors, restored
fullscreen behavior still require interactive Windows acceptance. The measured
development main-to-drawn-frame interval is recorded separately in T1.09 evidence.

Cloud defaults recognize configured cloud roots and common folder names; this is
a conservative local policy, not a census of every installed cloud client.
Unrecognized custom sync roots remain an open location-detection clause.

The T1.10 source candidate below now connects pairing and project sessions through
the real shared facade. An inbound listener opens only after an explicit pairing
or Share action with selected numeric endpoints. Firewall and route-metric changes
are never automatic. The instruction panel remains a Phase 2 placeholder.

Native compilation, generated bindings, integration tests and Windows UI checks
are owned by the parent build lane. No screenshot or runtime acceptance evidence
was produced in this worktree. The written JVM tests cover all saved placements,
cloud defaults, coordinate edits and 100 peer updates leaving camera bits intact.
Additional written regressions exercise delayed/duplicate refreshes, the
info-to-commit peer race, retained text drafts, revision-bound geometry and
offscreen pixels for affine pen widths, marker alpha and compound highlighter
fills. These new tests have not been executed in this worktree. Integration
requires the transport worktree's additive shared EditOptions precondition fields.

## T1.10 desktop session candidate

The app creates one session service with the existing installation DeviceId. The
native factory opens current-user DPAPI storage; identity mismatch is an error,
never a reason to replace the stored identity. Pairing keeps its listener alive
through code confirmation and requires deliberate comparison of the complete
fingerprint on both devices. QR bytes use the shared bounded `vw-pair:v1:` envelope,
are displayed only in the pairing sheet, and are cleared on success, dismissal,
expiry and shutdown. Code strings are not persisted or logged. No screenshots or
credential files are generated.

QR rendering adds exact `com.google.zxing:core:3.5.4`, Apache-2.0, with no new core
runtime dependencies. Primary sources: [release](https://github.com/zxing/zxing/releases/tag/zxing-3.5.4),
[tagged license](https://raw.githubusercontent.com/zxing/zxing/zxing-3.5.4/LICENSE),
and [tagged core POM](https://raw.githubusercontent.com/zxing/zxing/zxing-3.5.4/core/pom.xml).
The parent owns dependency-lock regeneration, license evidence and compilation.

The sheet exposes trust listing/revocation, real numeric local-address enumeration,
bounded native mDNS browsing and advertisement of the actual pairing listener.
Discovered endpoints are untrusted hints and never become DeviceIds. Local listen
and remote-connect fields are separate, copied before asynchronous work, and
ordered tether, Wi-Fi, then adb. Empty fields create no endpoint. Native project
role checks distinguish journal hosts from replicas; receive creates a new local
project after authenticated bounded bootstrap, without replacing existing folders.

Each project owns its link, status collector and at most one preview query. They
close before the project; app shutdown closes session children before the service.
Pending connection/receive requests are cancellable. Link, connection-status and
render identities guard late callbacks, including a query that returns after an
offline/reconnect transition. Monotonic ProjectChange sequences, latest fetch tickets and
a current visible-state check admit same-host-sequence optimistic changes while
refusing stale document responses. Export binding still uses the exact state hash.

The desktop Pen tool queues only real pointer positions, at most 512 waiting
samples and 100,000 total. Mouse pressure is explicitly 1; no predicted input is
sent. Native append success precedes preview transmission, with batches paced at
9 ms or slower, and live geometry is capped at 1,048,576 quantized vertices. An
overflow cancels the whole unfinished stroke. Once commit returns, accepted receipt
bookkeeping settles before volatile closure, even if cancellation or a preview
error follows. The canonical stroke remains wet until the accepted document is
ready. Single-object handle and width gestures,
plus rectangle/ellipse/line/arrow creation, stream through native preview APIs.
New shapes keep D-space geometry and identity transforms, preserving outline width
and arrow geometry. The final edit carries the same gesture identity; successful
closure follows its durable receipt, and other exits request cancellation. A window
deactivation cancels active input. Multi-object edits are commit-only because the
native preview contract binds one target per gesture.

Peer contours use the same renderer and one compound fill. Overlay publication is
bound to the project attachment, exact visible revision and link epoch; cancellation
or revision changes remove old overlays immediately from admission. Viewport sharing,
follow and outline are separate explicit controls, off by default. Peer edits never
change the camera. Diagnostics show actual native echo counts/RTT/offset when present;
they are not drawing-latency measurements.

Read-only tether route inspection runs on explicit receive and each observed tether
connect/reconnect. The Devices sheet now also contains the source candidate for
explicit Windows connection assistance described below. Automated firewall changes, S23/S Pen
measurements, real QR/device pairing, native drag/clipboard interoperability, frame
latency and flash-free wet/dry acceptance remain open. The central native/shared,
85-case desktop JVM and 65-case IN2019 app runs cover their recorded synthetic
and native-bound paths; actual paired app acceptance is still pending.

Twenty-six additional written JVM regressions cover installed-identity mismatch,
full-fingerprint consent, expiry/credential cleanup, link closure and delayed
previews, tether reconnect inspection, endpoint order/status words, camera
independence, accepted-only ink, cancellation/queue bounds, D-space shape parity,
optimistic revision order, project/link closure, gesture IDs and slider cancellation.
They also gate in-flight previews across offline/reconnect, retain accepted ink
receipts through preview-close failure/cancellation, and retire wet geometry only
after the corresponding saved document is prepared.
These cases are included in the central desktop suite described above. The
original source-author handoff executed no tests independently.

### Windows connection assistance source candidate

Nothing starts on opening Devices. The owner chooses a trusted installed local
`adb.exe`, explicitly requests the existing server's device inventory, selects
one exact device, and opts into recovery for explicit phone/PC TCP ports. The
tool is invoked only for its local `version` response. Recovery talks directly
to the already-running IPv4 loopback ADB server using protocol 41; it never starts
or restarts that server, resets a device, falls back to another device, removes
a mapping, or replaces an occupied phone port. A matching existing mapping has
unknown/shared ownership. Missing, offline, unauthorized, unavailable, refused
and contested states remain visible. The watcher uses a 200 ms polling delay and
a 600 ms cycle budget; these are scheduling targets, not measured replug timing.

Closing Devices keeps an explicitly enabled watcher running. Stop and app shutdown
cancel and settle its owned work. Reverse mappings remain after stop/exit because
ADB provides no atomic ownership-checked removal. The sheet reports whether this
watch created mappings and explains manual removal through the owner's own tool.
Choosing a project loopback listener remains a separate explicit action.

For the selected tether interface, Inspect reads the default-route table and
offers exact old/new automatic-mode and metric values only when a bounded safe
demotion is available. Apply and Revert are separate deliberate buttons. They
launch a fixed packaged helper through Windows `runas`; no route command, shell
script, administrator action, or firewall change runs during inspection. Native
preflight binds interface index/LUID, family and the complete observed default
route table. It refuses stale values before the setter and verifies the result
before offering an opaque inverse. Reverts run newest first and are never
automatic. A declined UAC prompt or proven pre-setter admission refusal keeps
the exact action retryable. Stale/uncertain consumed actions retain their original
metric values visibly; a later unrelated fix cannot erase them. They clear only
when a verified revert restores those intended fields or the owner explicitly
marks that recovery resolved. Receipts are in-memory for this app session;
unrelated route changes can invalidate them. Cancellation once a setter is in
flight reports an uncertain outcome and requires another inspection. Windows has no transaction spanning
route comparison and the metric setter, so a simultaneous external change can
still yield an uncertain result rather than a success claim.

Before any host binding, DPI call, or AWT window, `DesktopNativeRuntime` admits an
exact three-file SHA-256/size inventory and extracts the two DLLs and helper to
an owned hash-named directory below local application data. It checks and pins
the parent chain against redirection, creates files without overwriting, flushes
them, and publishes a readiness marker last. Existing caches must contain only
the exact marked inventory. Read handles prevent replacement while the JVM is
using their absolute UniFFI library paths. Each file is capped at 256 MiB, the
inventory at 512 MiB, and retained cache entries at 32. Close releases pins but
preserves cache files because Windows may keep DLLs loaded. Only a failed call's
verified newly created, never-loaded files are eligible for its narrow cleanup;
changed or preexisting entries are preserved. Review old marked caches only after
all app instances and helper processes have exited; no recursive startup cleanup
runs. An incomplete retained cache fails explicitly instead of being overwritten.

The additive host modules and required root integration are documented in
`host-win/crates/vw-host-ffi/CONNECTION_ASSISTANCE.md`. The Gradle task packages the
prebuilt helper and full binary inventory; it does not spawn Cargo. The root must
retain its separately added native-notices resource directory during this merge.
Fifteen new injected controller tests and eight private-file bootstrap tests are
written. They do not execute ADB, connect a socket, load task DLLs, prompt for UAC,
change a metric, or touch a real clipboard. Native unit tests also use pure
decisions and injected backends. These new tests have not been run here. Actual
server compatibility, physical recovery timing, generated binding compilation,
Windows DLL pins, UAC lifecycle and metric fix/revert acceptance remain pending.

## T1.11 desktop handoff candidate

File > Import and one-file native AWT drops stream PNG/JPEG/WebP into an owned
private file before `WorkbenchStreamingCore.createFile`; the desktop does not
allocate the complete encoded file or pass it across UniFFI. The native pipeline
still retains its bounded encoded input while processing raster regions. Files
are capped at the current carrier's
64 MiB, while the native image pipeline enforces the 50 MP, format and working
memory limits. The original is never changed or resized. A dropped source gets
its completion callback only after the private copy is flushed, allowing a source
app to release its temporary file. The actual AWT drop callback remains on the
stack through a secondary event loop until staging completes; file copying runs
off the UI thread. A 15-second staging deadline or window disposal rejects the
drop and requests cancellation without deleting files still owned by its worker.
Before a successful callback, the private copy gets a flushed recovery marker
(at most 2 KiB; source bytes at most 64 MiB). Subsequent decode or cancellation
failure preserves that original and reports its exact `input` path for retry.
Successful native project/asset acceptance permits staging cleanup. This makes
the native acknowledgment about retained bytes, not successful image decoding.
Recovery copies remain until deliberate recovery/cleanup and can accumulate;
the desktop does not silently delete an accepted drop to reclaim space.
MP4 imports use `attachFile` on the captured
project as an ordinary asset; no document or timeline is fabricated.

File > Paste image (Ctrl+V while the canvas owns focus) explicitly reads the host's
preferred PNG/DIB format. Text fields retain Ctrl+V. The paste dialog offers an
explicit untagged-DIB sRGB assumption. Clipboard payloads are bounded to the host's
16 MP / 32 MiB PNG / 64 MiB DIB limits; larger originals use file import. Nothing
reads the clipboard at startup or in response to clipboard notifications.

The export sheet offers clean/marked, full/selection-bounds/view-bounds, PNG8,
PNG16, JPEG quality, WebP lossless/lossy quality, explicit matte, sRGB conversion,
untagged-color assumption and depth reduction. Native `preflightFile` reports
typed dimension, pixel, alpha, depth, profile, memory, encoded and scratch limits;
marked-render admission is repeated by `exportTransfer`. Selection/view bounds
are clipped and rounded outwards in document pixels, without scaling.

An export request captures attachment, project, document, exact host sequence and
state hash, settings, and applicable selection/camera. Picker callbacks and queued
work retain that immutable request. The complete file receipt must match it before
readiness or publication. `EXPORT READY` is derived from the completed receipt and
current state, and disappears when any relevant binding changes. A stale native
receipt is discarded; the previous display bitmap is never an export source.

The native core first writes a complete private export. Save copies it in bounded
chunks to a new sibling, flushes it, checks the request again, and uses atomic
no-clobber hard-link creation for the chosen destination. Existing names are
preserved. A volume without this operation is refused rather than publishing a
partial file; Windows NTFS behavior still needs the root validation run. Only the
owned sibling is removed afterward. An already-published destination is never
rolled back during cancellation or cleanup errors.

Copy explicitly publishes the exact PNG and its prepared CF_DIBV5 companion
through the host service; source/revision/file hashes come from the completed
receipt and are verified again by the host. The sheet separately asks permission
for DIBV5 depth reduction or an untagged-color assumption. Non-PNG settings retain
Save support and visibly require PNG for copy/drag.

Prepare file drag obtains an owned native `DragFile` first. The actual AWT gesture
callback consumes that exact lease and starts `startDrag` synchronously with only
`javaFileListFlavor`. Its listener holds the lease through `dragDropEnd` and then
releases/destroys it; a failed start also releases it. An unused prepared lease is
retired on settings/project changes and shutdown. Native retention/orphan cleanup
keeps delayed receivers' PNG paths available after the gesture. No mouse gesture,
paste into another application, Enter or Send key is synthesized.
Preparation closes the options dialog and returns to the persistent transfer-shelf
drag button; that AWT component stays mounted and enabled throughout its active
gesture, even after the controller transfers the prepared lease to its listener.

Shared streaming calls already join native work on cancellation. The Windows
adapter adds an independent bounded producer, signals its operation token, and
awaits settlement before disposing inputs or undelivered native objects. Private
desktop workspaces verify a random ownership marker and absolute containment
before cleanup, and never follow links. Ordinary staging cleans up on completion,
cancellation and shutdown; accepted drops that later fail remain recoverable.
Interrupted process leftovers are also preserved for deliberate cleanup rather
than deleting an unverifiable directory on startup.

`DesktopTransferTest` contains injected controller and private-file regressions
for immutable picker requests, receipt/revision/settings/region readiness,
streamed import, durable drop ownership, exact completed-file copy, MP4 attachment,
drag lease handoff, no-clobber publication and stale-result cleanup. It uses no
real clipboard or drag target. Secondary-loop regressions exercise callback
stack lifetime, immediate/duplicate completion, timeout, disposal and late
callbacks; negative import tests prove accepted bytes survive failure/cancellation.
These sources have not been compiled or run here.
Root must integrate the frozen shared streaming/transfer contracts and reviewed
host-ffi subtree, regenerate bindings, and run the serialized JVM/native checks.
No additional dependency or build task was introduced by T1.11.

Windows AWT native drag/drop delivery, output-volume hard links, DIBV5/PNG paste
compatibility and real destination acceptance remain unverified. Claude desktop,
ChatGPT desktop/browser, an image editor and the owner's separate Claude Code
session still require guarded acceptance. Session/UI pairing integration and
cross-device MP4 transfer are independent open clauses; this source does not pass
those gates or Android/offline share acceptance.

Dependencies reuse coroutines 1.9.0, adding its Swing dispatcher under the same
[Apache-2.0 license](https://raw.githubusercontent.com/Kotlin/kotlinx.coroutines/1.9.0/LICENSE.txt).
Test-only [JUnit 4.13.2](https://github.com/junit-team/junit4/releases/tag/r4.13.2)
uses [EPL-1.0](https://raw.githubusercontent.com/junit-team/junit4/r4.13.2/LICENSE-junit.txt).
The shared module packages both native DLLs; desktop does not spawn a duplicate
Cargo build. All dependency lock updates belong to the parent integration lane.
