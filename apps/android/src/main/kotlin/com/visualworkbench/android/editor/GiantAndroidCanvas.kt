package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal const val GIANT_ANDROID_SURFACE_BYTES: ULong = 100_663_296uL
private const val GIANT_ANDROID_TILE_BYTES: Long = 96L * 1024L * 1024L

/** Physical pixels, already scaled by the window/camera. Four tightly packed
 * RGBA buffers use at most 80 MiB, leaving 16 MiB of the 96 MiB surface reservation
 * for known stride/buffer overhead. This is not driver/RSS acceptance. */
internal fun admitGiantAndroidSurface(width: Int, height: Int) {
    require(width in 1..8192 && height in 1..8192 && width.toLong() * height <= 5L * 1024L * 1024L)
}

/** Root MUST retain owner in its existing failed-retirement quarantine before
 * mapping this fixed diagnostic. No failed factory owner may be discarded. */
internal class GiantAndroidCreationFailure(internal val owner: GiantAndroidCanvas) :
    Exception("Giant renderer creation cleanup remains owned")

/** update/create/close run on Main; matchingFrame is also used by the actual
 * renderer with a root-supplied current immutable binding/view snapshot. */
internal class GiantAndroidCanvas private constructor(
    private val image: GiantImage,
    private val resources: SessionResources,
    private val binding: WorkflowBinding,
    private val epoch: Long,
    private val isCurrent: (WorkflowBinding, Long) -> Boolean,
    val content: GiantTileContent,
    private val assumeUntaggedSrgb: Boolean,
    private val retireSurface: suspend () -> Unit,
    private val changed: () -> Unit,
) {
    private val bitmapFactory = GiantAndroidBitmapFactory()
    private val closing = Mutex()
    private var surfaceCharge: SessionResourceCharge? = null
    @Volatile private var surfaceHeld = false
    private var viewport: GiantViewportSession<Bitmap>? = null
    private var observer: Job? = null
    @Volatile private var sealed = false
    @Volatile private var published = false
    @Volatile private var retiring = false
    @Volatile private var candidate: GiantVisibleFrame<Bitmap>? = null
    @Volatile private var latestView: GiantViewport? = null
    private var closed = false
    private var imageClosedWithoutSession = false

    val surfaceReserved: Boolean get() = published && !sealed && !retiring && surfaceHeld
    val status: GiantViewStatus get() {
        val current = viewport?.state?.value ?: GiantViewStatus()
        return if (sealed || retiring || !published || latestView == null) current.copy(ready = false) else current
    }
    val frame: GiantVisibleFrame<Bitmap>? get() {
        val view = latestView ?: return null
        return matchingFrame(binding, epoch, view, content)
    }

    private val backend = object : GiantRenderBackend<Bitmap> {
        override suspend fun create(pixels: GiantTilePixels): Bitmap = bitmapFactory.create(pixels)
        override fun present(frame: GiantVisibleFrame<Bitmap>?) {
            candidate = frame
            // A fresh publication is the only evidence that a retired session
            // continued. Parent cancellation/close never reopens surface admission.
            if (frame != null && !sealed) retiring = false
            changed()
        }
        override suspend fun retire() {
            retiring = true; candidate = null
            // Gate the release callback's successor creation BEFORE awaiting it.
            // changed may fail, but that must not skip actual retirement.
            try { changed() } finally { retireSurface() }
            bitmapFactory.releaseFailedAfterRetirement()
            // Stay gated through shared tile release and a possible terminal
            // session close. Only a later valid frame can reopen this owner.
        }
        override fun release(image: Bitmap): Unit = releaseGiantAndroidBitmap(image)
    }

    private fun install(scope: CoroutineScope) {
        val onMain = CoroutineScope(scope.coroutineContext + Dispatchers.Main.immediate)
        val session = GiantViewportSession(onMain, image, backend, binding, epoch,
            isCurrent, resources, GIANT_ANDROID_TILE_BYTES, content, assumeUntaggedSrgb)
        viewport = session
        observer = onMain.launch(start = CoroutineStart.UNDISPATCHED) {
            session.state.collect { if (published && !sealed) changed() }
        }
    }

    fun update(core: WorkbenchCore, camera: Camera, precision: PrecisionFrame) {
        check(android.os.Looper.myLooper() == android.os.Looper.getMainLooper())
        if (sealed || !published) return
        latestView = null // Hide old display even when new geometry refuses.
        try {
            require(camera.viewportWidth.isFinite() && camera.viewportHeight.isFinite() &&
                camera.viewportWidth > 0.0 && camera.viewportHeight > 0.0 &&
                camera.viewportWidth <= 8192.0 && camera.viewportHeight <= 8192.0)
            admitGiantAndroidSurface(kotlin.math.ceil(camera.viewportWidth).toInt(), kotlin.math.ceil(camera.viewportHeight).toInt())
            val view = giantViewport(core, camera, precision)
            latestView = view
            checkNotNull(viewport).request(view)
        } finally { if (latestView == null) changed() }
    }

    /** Root repeats this at actual draw, under its render-session fence, and
     * must not reuse the returned frame after that reader/pipeline retires. */
    fun matchingFrame(binding: WorkflowBinding, epoch: Long, viewport: GiantViewport,
        expectedContent: GiantTileContent = content): GiantVisibleFrame<Bitmap>? {
        val value = candidate ?: return null
        val state = status
        if (!surfaceReserved || !state.ready || value.generation != state.generation ||
            this.binding != binding || this.epoch != epoch || content != expectedContent ||
            value.content != expectedContent || latestView != viewport ||
            !isCurrent(binding, epoch) || !value.matches(binding, epoch, viewport)) return null
        // Recheck publication after callbacks/state reads; root owns the exact
        // draw/retirement lock, this read alone is not a graphics retirement ACK.
        val finalState = status
        return value.takeIf { candidate === it && surfaceReserved && finalState.ready &&
            finalState.generation == it.generation && it.content == expectedContent &&
            latestView == viewport && isCurrent(binding, epoch) }
    }

    suspend fun close(): Unit = withContext(NonCancellable + Dispatchers.Main.immediate) {
        closing.withLock {
            if (closed) return@withLock
            sealed = true; published = false; candidate = null; latestView = null
            var notificationFailure: Throwable? = null
            try { changed() } catch (error: Throwable) { notificationFailure = error }
            observer?.cancelAndJoin(); observer = null
            val session = viewport
            if (session != null) session.close()
            else if (!imageClosedWithoutSession) { image.close(); imageClosedWithoutSession = true }
            // A failed session/native/image close never credits the surface.
            surfaceCharge?.close(); surfaceCharge = null; surfaceHeld = false
            closed = true
            notificationFailure?.let { throw it }
        }
    }

    companion object {
        suspend fun create(scope: CoroutineScope, image: GiantImage, resources: SessionResources,
            binding: WorkflowBinding, epoch: Long, isCurrent: (WorkflowBinding, Long) -> Boolean,
            retireSurface: suspend () -> Unit, changed: () -> Unit,
            content: GiantTileContent = GiantTileContent.Source,
            assumeUntaggedSrgb: Boolean = false): GiantAndroidCanvas {
            // Allocate the cleanup owner BEFORE the first suspension; never lose
            // acquired resources to prompt cancellation of a dispatcher return.
            val owner = GiantAndroidCanvas(image, resources, binding, epoch, isCurrent,
                content, assumeUntaggedSrgb, retireSurface, changed)
            try {
                currentCoroutineContext().ensureActive()
                scope.coroutineContext[Job]?.ensureActive()
                withContext(NonCancellable + Dispatchers.Main.immediate) {
                    require(epoch >= 0 && binding.projectId.isNotEmpty() && binding.documentId.isNotEmpty())
                    require(binding.stateHash.length == 64 && binding.stateHash.all { it in "0123456789abcdef" })
                    check(isCurrent(binding, epoch))
                    owner.surfaceCharge = resources.reserveForeign(GIANT_ANDROID_SURFACE_BYTES)
                    owner.surfaceHeld = true
                    owner.install(scope)
                }
                currentCoroutineContext().ensureActive()
                scope.coroutineContext[Job]?.ensureActive()
                check(isCurrent(binding, epoch))
                owner.published = true
                return owner
            } catch (error: Throwable) {
                // Preserve settlement across close's Main dispatcher return to
                // a cancelled caller; only an actual cleanup failure owns a quarantine.
                try { withContext(NonCancellable) { owner.close() } }
                catch (_: Throwable) { throw GiantAndroidCreationFailure(owner) }
                throw error
            }
        }
    }
}

/** Only D-space placement. Root supplies the canonical base/precision transform
 * and holds its actual draw-reader fence across this call. */
internal fun drawGiantAndroidTiles(canvas: Canvas, frame: GiantVisibleFrame<Bitmap>) {
    for (tile in frame.tiles) {
        val p = tile.pixels; val bitmap = tile.image
        require(p.width in 1u..256u && p.height in 1u..256u && p.documentWidth > 0u && p.documentHeight > 0u)
        require(p.x.toULong() + p.documentWidth <= 1_048_576uL && p.y.toULong() + p.documentHeight <= 1_048_576uL)
        check(!bitmap.isRecycled && bitmap.width == p.width.toInt() && bitmap.height == p.height.toInt())
    }
    // Android Q+ constructors enable filtering even with flags=0. Clear it
    // explicitly so document tiles retain nearest sampling at their edges.
    val paint = Paint().apply { isFilterBitmap = false }
    for (tile in frame.tiles) {
        val p = tile.pixels; val bitmap = tile.image
        canvas.drawBitmap(bitmap, Rect(0, 0, bitmap.width, bitmap.height),
            RectF(p.x.toFloat(), p.y.toFloat(), (p.x.toULong() + p.documentWidth).toFloat(),
                (p.y.toULong() + p.documentHeight).toFloat()), paint)
    }
}
