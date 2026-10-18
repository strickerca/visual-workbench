# Parallel renderer leaves and delivery ownership

Root keeps public/common/FFI contracts, both EditorControllers, Android
CanvasSurface/EditorDrawing/WorkbenchApp, Desktop EditorCanvas/Main and native
registration. Image owner keeps the native provider and pure-stream budget work
already assigned. Do not overlap either lane or run a build while image owns the
single heavy slot. Source/tests can proceed concurrently.

## Shared source ready for both renderer lanes

GiantViewportSession now requires SessionResources after isCurrent. Before
image.tile(address, foreignCharge), it reserves TILE_CHARGE from the endpoint's
actual native pool; local view counters cannot grant admission. It keeps the
charge beside the image through real pipeline retirement. Image and reservation
release phases are distinct: a failed charge release never double-destroys an
already-released platform image. Backend.create failures also require retirement
before the reservation can be credited; retain privately owned failed creations
until retire actually settles them.

Use common giantViewport(core, camera, precision) for the exact request envelope.
It includes precision pane footprints at their highest physical scale, without
applying DPI again. GiantVisibleFrame.matches(binding, epoch, viewport) is the
full revision/attachment/view fence; actual platform draw also compares the
latest viewport generation and exact owner frame. Existing input/precision/eraser/
selection controllers remain authoritative.

Root's pending native SessionResourcePool wraps a process-wide SessionBudget
(256 MiB Android,512 MiB elsewhere); replacement sessions cannot reset old live
charges. sessionResources() is the common adapter. Both renderer surfaces and
all tile foreign copies MUST use it. Driver/RSS acceptance remains unmeasured.

## Producer -> ABI -> foreign lease (image-owner coordination)

The image owner's private delivery retains its outgoing MemoryCharge. Root will
consume its committed exact delivery result hook; do not discard that charge
when projecting pixels into the byte-only DTO. The chosen root ABI shape is
GiantTileDelivery { pixels: GiantTilePixels, native_lease: Arc<SessionResourceCharge> }.
The opaque native lease retains the outgoing charge AND an extra bounded ABI
serialization copy before UniFFI starts marshalling. GiantTilePixels itself stays
byte-only and never derives Debug. No raw pixels are printed.

The common NativeGiantImage adapter requires a live pre-reserved foreign charge
before tile admission. It joins cancellation and retains the native delivery
lease through complete generated-reply/RustBuffer decoding. Only then can it
close the temporary native delivery lease: the original global foreign charge
continues to own ByteArray/CPU bitmap/GPU texture until actual renderer retirement.
Thus there is deliberate overlap and no uncharged handoff interval. Late cancelled
native delivery must close its opaque lease before returning cancellation. Root
will register the native budget/ABI wrappers together with the worker's exact
provider hook and generate bindings once for that coherent phase. Current native
adapter source is not compiled against final generated methods yet.

## Proposed Android renderer leaf owner

Exclusive NEW files:
- apps/android/src/main/kotlin/com/visualworkbench/android/editor/GiantAndroidCanvas.kt
- apps/android/src/main/kotlin/com/visualworkbench/android/editor/GiantAndroidBitmap.kt
- apps/android/src/test/kotlin/com/visualworkbench/android/editor/GiantAndroidBoundaryTest.kt
- apps/android/src/androidTest/kotlin/com/visualworkbench/android/editor/GiantAndroidCanvasInstrumentedTest.kt

No existing root/platform files or manifests. Adapt only relevant recovered
bitmap/draw/owner leaves; no controller postimages or second renderer registry.
Root already has a joined CanvasFrontBufferedRenderer release callback and will
pass that real retirement function into this owner. Do not acknowledge retirement
by clearing a frame reference, hiding a SurfaceControl or guessing a timeout.

Authoritative private platform API to implement:

    internal class GiantAndroidCanvas {
      companion object {
        suspend fun create(scope: CoroutineScope, image: GiantImage,
          resources: SessionResources, binding: WorkflowBinding, epoch: Long,
          isCurrent: (WorkflowBinding,Long)->Boolean,
          retireSurface: suspend ()->Unit, changed: ()->Unit): GiantAndroidCanvas
      }
      val frame: GiantVisibleFrame<Bitmap>?
      val status: GiantViewStatus
      val surfaceReserved: Boolean
      fun update(core: WorkbenchCore, camera: Camera, precision: PrecisionFrame)
      fun matchingFrame(binding: WorkflowBinding, epoch: Long,
          viewport: GiantViewport): GiantVisibleFrame<Bitmap>?
      suspend fun close()
    }
    internal fun admitGiantAndroidSurface(width: Int, height: Int)
    internal fun drawGiantAndroidTiles(canvas: Canvas,
        frame: GiantVisibleFrame<Bitmap>)

Factory takes ownership of image and joins it on every failed/cancelled return.
Reserve a96MiB surface allowance from resources before renderer allocation;
admit physical sides<=8192 and at most5Mi pixels, leaving stride/known buffer
headroom rather than using the allowance to its last tight-packed byte. No DPI
multiplication: Camera viewport is already physical. Root will retire the old
pipeline and gate CanvasSurface creation on surfaceReserved. Mark sealed/not ready
before retiring, so the release callback cannot create an unreserved successor.
Keep the surface charge until actual session/render retirement succeeds; a failed
owner remains held for root quarantine, including native/image close failures.

Use the injected real retireSurface as GiantRenderBackend.retire. Bitmap creation
must be cancellation-safe across Default->Main handoff, verify actual rowBytes/
allocationByteCount against the charged per-tile copies before upload, and admit
only opaque RGBA8 sRGB. The draw helper draws D-space tile rectangles only; root
supplies the current canvas transform and precision passes. matchingFrame must
check isCurrent, full binding/epoch/view, state.ready and latest generation.

Write static bounds and ownership tests, plus synthetic instrumentation for
bitmap bytes/stride, cancellation, actual renderer-release ordering and failed
retirement retention where applicable. No device tests without a separate grant.
No product or physical acceptance is inferred from compilation/fake callbacks.

## Proposed Desktop renderer leaf owner

Exclusive NEW files:
- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/GiantCanvasHost.kt
- apps/desktop/src/test/kotlin/com/visualworkbench/desktop/GiantCanvasBoundaryTest.kt

No EditorController/EditorCanvas/Main/native-runtime/Gradle edits. Adapt the
recovered owned ComposePanel/Skia context, not a second input or document system.
Use injected content instead of calling an old EditorCanvas signature.

    internal class GiantBitmap /* exposes its ImageBitmap, owns native Image */
    internal class GiantShortcutRoute<E> /* current handler; dispatch once; seal */
    internal class GiantDesktopView {
      companion object {
        suspend fun create(scope: CoroutineScope, image: GiantImage,
          resources: SessionResources, binding: WorkflowBinding, epoch: Long,
          isCurrent: (WorkflowBinding,Long)->Boolean, cancelInput: ()->Unit,
          changed: (GiantViewStatus)->Unit): GiantDesktopView
      }
      val status: GiantViewStatus
      val surfaceReserved: Boolean
      fun update(core: WorkbenchCore, camera: Camera, precision: PrecisionFrame)
      fun matchingFrame(binding: WorkflowBinding, epoch: Long,
          viewport: GiantViewport): GiantVisibleFrame<GiantBitmap>?
      suspend fun close()
    }
    @Composable internal fun GiantEditorCanvas(view: GiantDesktopView,
      onShortcut: (KeyEvent)->Boolean,
      content: @Composable (GiantVisibleFrame<GiantBitmap>?)->Unit)
    internal fun DrawScope.drawGiantTiles(frame: GiantVisibleFrame<GiantBitmap>)
    internal fun giantSurfaceAdmitted(width:Int,height:Int,
        scaleX:Double,scaleY:Double):Boolean

Factory owns image on entry and settles every failed/cancelled return. Reserve
128MiB of surface allowance from the SAME resources before creating a drawing
context. Surface admission uses actual AWT logical dimensions and per-monitor
scale, sides<=8192 and physical pixels<=16Mi. The camera/frame request is already
physical; do not multiply its geometry by DPI again. Verify raster row bytes and
known tile storage against TILE_CHARGE; no encoded PNG intermediary is needed.
Unknown driver/private surface overhead remains a target-validation gap.

Retire means detach AND dispose the owned ComposePanel/recorded context on EDT
before releasing any epoch image or surface charge. Set panel=null before removal
to avoid reentrant retirement. Failed creation/release must retain ownership and
charges; repeated close must never counterfeit recovery or free a live reference.
An ordinary viewport request hides the prior frame immediately, but its images
stay charged until real context retirement. Content uses current injected root
callbacks; shortcut routing forwards once to the same focus/modal-aware Window
handler and never globally redispatches AWT events. matchingFrame has the same
full binding/epoch/view/latest-generation requirements as Android.

Write pure surface/DPI/shortcut and ownership tests. Headless compilation is not
actual ComposePanel/GPU retirement proof. Parent will return precise Windows/runtime
limits; do not install tooling, call providers or make hardware/signing changes.
