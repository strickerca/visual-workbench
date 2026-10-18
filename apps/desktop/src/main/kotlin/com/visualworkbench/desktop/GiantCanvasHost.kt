@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
package com.visualworkbench.desktop

// Adapted from the owned ComposePanel/raster/draw leaves preserved at
// 473395197f6437586a8b0c05fa8cd35aa6800441, recovery/source-candidates/desktop.
// Shared contracts follow GIANT_RENDERER_PARALLEL_HANDOFF_20261018.md at 8d95427.

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.awt.ComposePanel
import androidx.compose.ui.awt.SwingPanel
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import com.visualworkbench.shared.*
import java.awt.Dimension
import javax.swing.JPanel
import javax.swing.SwingUtilities
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.jetbrains.skia.ColorAlphaType
import org.jetbrains.skia.ColorSpace
import org.jetbrains.skia.ColorType
import org.jetbrains.skia.Image
import org.jetbrains.skia.ImageInfo
import kotlin.math.ceil

internal const val GIANT_DESKTOP_SURFACE_BYTES: ULong = 128uL * 1024uL * 1024uL
private const val TILE_RGBA_BYTES: Long = 256L * 256L * 4L
private const val TILE_BOOKKEEPING_BYTES: Long = 16L * 1024L

/** A nested scene calls the current Window handler once. No AWT redispatch. */
internal class GiantShortcutRoute<E> {
    private var handler: ((E) -> Boolean)? = null
    private var sealed = false
    private var dispatching = false
    fun update(value: (E) -> Boolean) { if (!sealed) handler = value }
    fun dispatch(event: E): Boolean {
        if (sealed || dispatching) return false
        val current = handler ?: return false
        dispatching = true
        return try { current(event) } finally { dispatching = false }
    }
    fun clear() { handler = null }
    fun seal() { sealed = true; handler = null }
}

internal fun giantSurfaceAdmitted(width: Int, height: Int, scaleX: Double, scaleY: Double): Boolean {
    if (width <= 0 || height <= 0 || !scaleX.isFinite() || !scaleY.isFinite() || scaleX <= 0 || scaleY <= 0) return false
    val w = ceil(width.toDouble() * scaleX)
    val h = ceil(height.toDouble() * scaleY)
    return w <= 8192 && h <= 8192 && w * h <= 16.0 * 1024 * 1024
}

/** Includes the borrowed reply, actual raster stride, a full RGBA8 tile texture
 * and the session's per-image bookkeeping. No PNG or uncharged readback copy. */
internal fun giantRasterAdmitted(width: Int, height: Int, rowBytes: Long, replyBytes: Long): Boolean {
    if (width !in 1..256 || height !in 1..256 || rowBytes < width * 4L ||
        replyBytes != width * height * 4L || rowBytes > TILE_RGBA_BYTES) return false
    return replyBytes + rowBytes * height + TILE_RGBA_BYTES + TILE_BOOKKEEPING_BYTES <= GiantViewportSession.TILE_CHARGE
}

/** A failed destructor has an unknown outcome. Retain it and never invoke it a
 * second time or credit its reservation by pretending a retry recovered it. */
internal class GiantReleaseAction(private val destroy: () -> Unit) {
    private var released = false
    private var failure: Throwable? = null
    fun close() {
        failure?.let { throw it }
        if (released) return
        try { destroy(); released = true } catch (error: Throwable) { failure = error; throw error }
    }
}

/** One recorded graphics epoch, used on EDT. Detach can reenter framework
 * callbacks, so the public slot is cleared before either retirement operation. */
internal class GiantPanelEpoch<P : Any>(private val cancelInput: () -> Unit,
    private val detach: (P) -> Unit, private val dispose: (P) -> Unit) {
    var current: P? = null; private set
    private var retained: P? = null
    var retiring: Boolean = false; private set
    var failure: Throwable? = null; private set
    fun install(value: P) {
        check(current == null && retained == null && !retiring && failure == null)
        current = value
    }
    fun retire() {
        failure?.let { throw it }
        check(!retiring) { "Graphics retirement reentered before acknowledgement" }
        val old = current ?: return
        current = null; retained = old; retiring = true
        try {
            cancelInput(); detach(old); dispose(old)
            retained = null
        } catch (error: Throwable) { failure = error; throw error }
        finally { retiring = false }
    }
}

/** The single view close joins graphics/tile/native ownership before crediting
 * the surface. Concurrent/repeated calls join, and failures remain quarantined. */
internal class GiantDesktopRetirement(private val stop: () -> Unit,
    private val settle: suspend () -> Unit, private val credit: () -> Unit) {
    private val gate = Mutex()
    @Volatile var sealed: Boolean = false; private set
    private var completed = false
    private var failure: Throwable? = null
    suspend fun close(): Unit = withContext(NonCancellable) {
        gate.withLock {
            failure?.let { throw it }
            if (completed) return@withLock
            sealed = true
            try { stop(); settle(); credit(); completed = true }
            catch (error: Throwable) { failure = error; throw error }
        }
    }
}

internal fun <T : Any> giantFrameCurrent(frame: GiantVisibleFrame<T>?, owned: GiantVisibleFrame<T>?,
    binding: WorkflowBinding, epoch: Long, viewport: GiantViewport?, status: GiantViewStatus,
    admitted: Boolean, current: Boolean): GiantVisibleFrame<T>? = frame?.takeIf {
    admitted && current && viewport != null && it === owned && status.ready &&
        it.generation == status.generation && it.matches(binding, epoch, viewport)
}

internal class GiantRasterOwner(val native: Image) {
    private val release = GiantReleaseAction { native.close() }
    fun close() = release.close()
}

internal class GiantBitmap(val image: ImageBitmap, private val raster: GiantRasterOwner,
    private val canDraw: (GiantVisibleFrame<GiantBitmap>) -> Boolean) {
    @Volatile private var released = false
    fun drawable(frame: GiantVisibleFrame<GiantBitmap>): Boolean = !released && canDraw(frame)
    fun release() { raster.close(); released = true }
}

/** The coordinator must keep owner in its existing quarantine if failed factory
 * cleanup cannot prove retirement. There is no additional renderer registry. */
internal class GiantDesktopCreationFailure(val owner: GiantDesktopView,
    val creationFailure: Throwable, cleanupFailure: Throwable) :
    Exception("Giant-image creation could not finish retirement", cleanupFailure)

private class GiantCanvasHost(private val cancelInput: () -> Unit,
    private val reserved: () -> Boolean,
    private val drawAllowed: (GiantVisibleFrame<GiantBitmap>) -> Boolean,
    private val notify: () -> Unit) : GiantRenderBackend<GiantBitmap> {
    private var presentation by mutableStateOf<GiantVisibleFrame<GiantBitmap>?>(null)
    private var observedStatus by mutableStateOf(GiantViewStatus())
    private var content by mutableStateOf<(@Composable (GiantVisibleFrame<GiantBitmap>?) -> Unit)?>(null)
    @Volatile var frame: GiantVisibleFrame<GiantBitmap>? = null; private set
    @Volatile var surfaceAdmitted: Boolean = false; private set
    private var stopped = false
    private var detaching = false
    private var unknownCreation: Throwable? = null
    private var platformFailure: Throwable? = null
    private val failedRasters = mutableListOf<GiantRasterOwner>()
    private val shortcuts = GiantShortcutRoute<KeyEvent>()
    private var currentShortcut: ((KeyEvent) -> Boolean)? = null
    private val epoch = GiantPanelEpoch<ComposePanel>(cancelInput,
        { container.remove(it) }, { it.dispose() })
    val container: JPanel = object : JPanel(null) {
        override fun doLayout() {
            checkEdt()
            val transform = graphicsConfiguration?.defaultTransform
            val next = !detaching && isDisplayable && transform != null && giantSurfaceAdmitted(width, height, transform.scaleX, transform.scaleY)
            val changed = next != surfaceAdmitted
            surfaceAdmitted = next
            if (!next) {
                try { retireNow() } catch (error: Throwable) { platformFailure = error }
            } else {
                try { ensurePanel(); epoch.current?.setBounds(0, 0, width, height) }
                catch (error: Throwable) { platformFailure = error; surfaceAdmitted = false }
            }
            if (changed || platformFailure != null) notify()
        }
        override fun addNotify() { super.addNotify(); detaching = false }
        override fun removeNotify() {
            detaching = true
            surfaceAdmitted = false
            // A nested removal is not a separate retirement acknowledgement.
            try { if (!epoch.retiring) retireNow() }
            catch (error: Throwable) { platformFailure = error }
            finally {
                try { super.removeNotify() }
                catch (error: Throwable) { platformFailure = error; throw error }
            }
            notify()
        }
    }.apply {
        preferredSize = Dimension(640, 480)
        isOpaque = true; background = java.awt.Color(12, 17, 24)
    }
    init { checkEdt() }

    fun updateContent(handler: (KeyEvent) -> Boolean,
        render: @Composable (GiantVisibleFrame<GiantBitmap>?) -> Unit) {
        checkEdt()
        if (stopped) return
        currentShortcut = handler; shortcuts.update(handler); content = render
        ensurePanel()
    }
    fun statusChanged(status: GiantViewStatus) { checkEdt(); observedStatus = status; ensurePanel() }
    fun failed(): Boolean = platformFailure != null || unknownCreation != null || epoch.failure != null
    fun stop() { checkEdt(); stopped = true; frame = null; presentation = null; shortcuts.seal(); currentShortcut = null }

    private fun ensurePanel() {
        checkEdt()
        if (stopped || detaching || !container.isDisplayable || !reserved() || !surfaceAdmitted ||
            epoch.retiring || epoch.current != null || frame == null || failed()) return
        var created: ComposePanel? = null
        try {
            created = ComposePanel()
            epoch.install(created) // Ownership precedes content/attachment calls.
            created.isDisposeOnRemove = false // This owner disposes explicitly after detach.
            currentShortcut?.let(shortcuts::update)
            created.setContent {
                val candidate = presentation
                val ready = observedStatus.ready
                Box(Modifier.fillMaxSize().onPreviewKeyEvent(shortcuts::dispatch)) {
                    content?.invoke(candidate?.takeIf { ready && drawAllowed(it) })
                }
            }
            container.add(created); container.revalidate(); container.repaint()
        } catch (error: Throwable) {
            // No returned context means its destruction cannot be demonstrated.
            if (created == null) unknownCreation = error
            throw error
        }
    }
    override suspend fun create(pixels: GiantTilePixels): GiantBitmap {
        checkEdt(); currentCoroutineContext().ensureActive()
        check(!stopped && reserved() && !failed())
        val width = pixels.width.toInt(); val height = pixels.height.toInt()
        val rowBytes = width * 4L
        require(giantRasterAdmitted(width, height, rowBytes, pixels.rgba.size.toLong()))
        require((3 until pixels.rgba.size step 4).all { pixels.rgba[it] == (-1).toByte() })
        val info = ImageInfo(width, height, ColorType.RGBA_8888, ColorAlphaType.OPAQUE, ColorSpace.sRGB)
        require(info.minRowBytes.toLong() == rowBytes && info.isRowBytesValid(rowBytes))
        var raster: GiantRasterOwner? = null
        var delivered = false
        try {
            // Synchronous bounded raster creation avoids a cancellable dispatcher
            // return losing a newly allocated native image before adoption.
            raster = GiantRasterOwner(Image.makeRaster(info, pixels.rgba, rowBytes.toInt()))
            val actual = raster.native.imageInfo
            require(actual.width == width && actual.height == height && actual.colorType == ColorType.RGBA_8888 &&
                actual.colorAlphaType == ColorAlphaType.OPAQUE && actual.colorSpace?.isSRGB == true &&
                actual.minRowBytes.toLong() == rowBytes)
            val value = GiantBitmap(raster.native.toComposeImageBitmap(), raster, drawAllowed)
            currentCoroutineContext().ensureActive()
            delivered = true
            return value
        } catch (error: Throwable) {
            if (raster == null) unknownCreation = error
            throw error
        } finally {
            // Even a never-presented failed image waits for real epoch retirement.
            if (!delivered) raster?.let(failedRasters::add)
        }
    }
    override fun present(frame: GiantVisibleFrame<GiantBitmap>?) {
        checkEdt(); this.frame = if (stopped) null else frame; presentation = this.frame
        ensurePanel()
    }
    private fun retireNow() {
        checkEdt()
        epoch.retire() // Clears panel before detach; disposal is the actual fence.
        unknownCreation?.let { throw it }
        platformFailure?.let { throw it }
        val iterator = failedRasters.iterator()
        while (iterator.hasNext()) { iterator.next().close(); iterator.remove() }
        shortcuts.clear()
        container.revalidate(); container.repaint()
    }
    override suspend fun retire(): Unit = withContext(NonCancellable + Dispatchers.Main.immediate) { retireNow() }
    override fun release(image: GiantBitmap) { checkEdt(); image.release() }
}

internal class GiantDesktopView private constructor(private val image: GiantImage,
    private val resources: SessionResources, private val binding: WorkflowBinding, private val epoch: Long,
    private val isCurrent: (WorkflowBinding, Long) -> Boolean, private val cancelInput: () -> Unit,
    private val changed: (GiantViewStatus) -> Unit) {
    private var surface: SessionResourceCharge? = null
    private var host: GiantCanvasHost? = null
    private var session: GiantViewportSession<GiantBitmap>? = null
    private var observation: Job? = null
    @Volatile private var viewport: GiantViewport? = null
    private var requested: GiantViewport? = null
    private var input: Triple<Camera, MagnifierPane?, MagnifierPane?>? = null
    private var refused = false
    private val retirement = GiantDesktopRetirement(
        stop = { host?.stop() },
        settle = {
            observation?.cancelAndJoin()
            val active = session
            if (active != null) active.close()
            else { host?.retire(); image.close() }
        },
        credit = { surface?.close(); surface = null },
    )
    // Admission flag only. A sealed, failed owner still retains its charge and
    // must stay in the coordinator's quarantine until close succeeds.
    val surfaceReserved: Boolean get() = !retirement.sealed && surface != null
    val status: GiantViewStatus get() {
        val state = session?.state?.value ?: GiantViewStatus()
        val pipeline = host
        val message = when {
            pipeline?.failed() == true -> "The giant-image drawing context could not retire. Its resources remain reserved."
            refused -> "This giant-image viewport exceeds the admitted display surface budget. Reduce the window size."
            else -> state.message
        }
        return state.copy(ready = state.ready && surfaceReserved && !refused && pipeline?.surfaceAdmitted == true &&
            pipeline.frame?.let { matchingFrame(binding, epoch, it.view) === it } == true, message = message)
    }

    companion object {
        suspend fun create(scope: CoroutineScope, image: GiantImage, resources: SessionResources,
            binding: WorkflowBinding, epoch: Long, isCurrent: (WorkflowBinding, Long) -> Boolean,
            cancelInput: () -> Unit, changed: (GiantViewStatus) -> Unit): GiantDesktopView {
            val owner = GiantDesktopView(image, resources, binding, epoch, isCurrent, cancelInput, changed)
            try {
                // Store ownership inside NonCancellable before returning across
                // dispatchers, including a pre-cancelled creator/parent scope.
                withContext(NonCancellable + Dispatchers.Main.immediate) {
                    owner.surface = resources.reserveForeign(GIANT_DESKTOP_SURFACE_BYTES)
                    scope.coroutineContext.ensureActive()
                    check(isCurrent(binding, epoch))
                    val backend = GiantCanvasHost(cancelInput, { owner.surfaceReserved },
                        { candidate -> owner.viewport?.let { owner.matchingFrame(binding, epoch, it) === candidate } == true },
                        { changed(owner.status) })
                    owner.host = backend
                    val uiScope = CoroutineScope(scope.coroutineContext + Dispatchers.Main.immediate)
                    val active = GiantViewportSession(uiScope, image, backend, binding, epoch, isCurrent, resources)
                    owner.session = active
                    owner.observation = uiScope.launch(start = CoroutineStart.UNDISPATCHED) {
                        active.state.collect { value -> backend.statusChanged(value); changed(owner.status) }
                    }
                }
                currentCoroutineContext().ensureActive()
                return owner
            } catch (error: Throwable) {
                try { owner.close() }
                catch (cleanup: Throwable) { throw GiantDesktopCreationFailure(owner, error, cleanup) }
                throw error
            }
        }
    }

    fun update(core: WorkbenchCore, camera: Camera, precision: PrecisionFrame) {
        checkEdt()
        if (retirement.sealed) return
        val active = session ?: return
        val nextInput = Triple(camera, precision.lens, precision.loupe)
        if (input != nextInput) { viewport = null; host?.present(null); input = nextInput }
        val next = try { giantViewport(core, camera, precision) } // Camera is already physical.
        catch (error: Throwable) {
            viewport = null; refused = true; host?.present(null); changed(status)
            throw error
        }
        val physical = giantSurfaceAdmitted(ceil(camera.viewportWidth).toInt(), ceil(camera.viewportHeight).toInt(), 1.0, 1.0)
        viewport = if (physical) next else null
        refused = !physical
        if (!physical) { host?.present(null); changed(status); return }
        if (next == requested && host?.frame == null && active.state.value.ready) active.retry()
        else active.request(next)
        requested = next
    }
    fun matchingFrame(binding: WorkflowBinding, epoch: Long, viewport: GiantViewport): GiantVisibleFrame<GiantBitmap>? {
        val owned = host?.frame
        return giantFrameCurrent(owned, owned, binding, epoch, viewport, session?.state?.value ?: GiantViewStatus(),
            surfaceReserved && !refused && host?.surfaceAdmitted == true && host?.failed() == false && this.viewport == viewport,
            this.binding == binding && this.epoch == epoch && isCurrent(binding, epoch))
    }
    internal fun container(): JPanel = checkNotNull(host).container
    internal fun content(handler: (KeyEvent) -> Boolean, value: @Composable (GiantVisibleFrame<GiantBitmap>?) -> Unit) {
        checkNotNull(host).updateContent(handler, value)
    }
    suspend fun close(): Unit = withContext(NonCancellable + Dispatchers.Main.immediate) { retirement.close() }
}

@Composable internal fun GiantEditorCanvas(view: GiantDesktopView, onShortcut: (KeyEvent) -> Boolean,
    content: @Composable (GiantVisibleFrame<GiantBitmap>?) -> Unit) {
    val currentContent by rememberUpdatedState(content)
    val currentShortcut by rememberUpdatedState(onShortcut)
    SwingPanel(factory = { view.container() }, update = {
        view.content({ event -> currentShortcut(event) }) { frame -> currentContent(frame) }
    }, modifier = Modifier.fillMaxSize())
}

internal fun DrawScope.drawGiantTiles(frame: GiantVisibleFrame<GiantBitmap>) {
    // Repeat the full live owner/generation fence at actual draw, not only when
    // a composable captured its frame. A copied/stale frame never gains a permit.
    if (frame.tiles.any { !it.image.drawable(frame) }) return
    for (tile in frame.tiles) {
        if (!tile.image.drawable(frame)) return
        val p = tile.pixels
        withTransform({
            translate(p.x.toFloat(), p.y.toFloat())
            scale(p.documentWidth.toFloat() / p.width.toFloat(), p.documentHeight.toFloat() / p.height.toFloat(), Offset.Zero)
        }) {
            drawImage(tile.image.image, srcOffset = IntOffset.Zero, srcSize = IntSize(p.width.toInt(), p.height.toInt()),
                dstOffset = IntOffset.Zero, dstSize = IntSize(p.width.toInt(), p.height.toInt()), filterQuality = FilterQuality.None)
        }
    }
}

private fun checkEdt() { check(SwingUtilities.isEventDispatchThread()) { "Giant drawing ownership requires EDT" } }
