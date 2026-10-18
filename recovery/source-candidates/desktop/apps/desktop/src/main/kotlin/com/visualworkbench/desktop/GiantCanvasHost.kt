@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.material.MaterialTheme
import androidx.compose.material.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.awt.ComposePanel
import androidx.compose.ui.awt.SwingPanel
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import java.awt.Dimension
import javax.swing.JPanel
import javax.swing.SwingUtilities
import org.jetbrains.skia.ColorAlphaType
import org.jetbrains.skia.ColorSpace
import org.jetbrains.skia.ColorType
import org.jetbrains.skia.Image
import org.jetbrains.skia.ImageInfo
import kotlin.math.ceil

/** The nested Compose scene forwards exactly one event to the same current
 * modal/focus-aware handler as the outer Window. It never redispatches globally. */
internal class GiantShortcutRoute<E> {
    private var handler:((E)->Boolean)?=null
    private var sealed=false
    fun update(value:(E)->Boolean){if(!sealed)handler=value}
    fun dispatch(value:E):Boolean=if(sealed)false else handler?.invoke(value)?:false
    fun clear(){handler=null}
    fun seal(){sealed=true;handler=null}
}

internal class GiantBitmap(val image: ImageBitmap, private val native: Image) {
    fun release() { native.close() }
}
/** Admission uses AWT logical bounds times the actual per-monitor transform.
 * Two RGBA surfaces are reserved below the 128 MiB surface allowance. The
 * physical Canvas callback remains authoritative for the native camera. */
internal fun giantSurfaceAdmitted(width: Int, height: Int, scaleX: Double, scaleY: Double): Boolean {
    if (width <= 0 || height <= 0 || !scaleX.isFinite() || !scaleY.isFinite() || scaleX <= 0 || scaleY <= 0) return false
    val w = ceil(width.toDouble() * scaleX); val h = ceil(height.toDouble() * scaleY)
    return w <= 8192 && h <= 8192 && w * h <= 16.0 * 1024 * 1024
}

/** A complete giant editor canvas has its own Compose/Skia context. Closing a
 * raster Image alone cannot evict references retained in a recorded picture.
 * We detach/dispose the owning ComposePanel before crediting ANY epoch tiles. */
internal class GiantCanvasHost(private val controller: EditorController, initial: EditorState) : GiantRenderBackend<GiantBitmap> {
    private var current by mutableStateOf(initial)
    private var frame by mutableStateOf<GiantVisibleFrame<GiantBitmap>?>(null)
    private var panel: ComposePanel? = null
    private var stopped = false
    private val shortcutRoute=GiantShortcutRoute<KeyEvent>()
    fun shortcuts(value:(KeyEvent)->Boolean){shortcutRoute.update(value)}
    val container = object : JPanel(null) {
        override fun doLayout() {
            val transform = graphicsConfiguration?.defaultTransform
            if (!giantSurfaceAdmitted(width, height, transform?.scaleX ?: 1.0, transform?.scaleY ?: 1.0)) {
                retireNow()
                controller.report("This giant-image viewport exceeds the admitted display surface budget. Reduce the window size.")
                return
            }
            ensurePanel(); panel?.setBounds(0, 0, width, height)
        }
        override fun removeNotify() {
            // The outer composition can disappear without a controller close.
            // Detach its renderer immediately; native/image ownership is still
            // retained by the session until its joined close.
            retireNow(); super.removeNotify()
        }
    }.apply { preferredSize = Dimension(640, 480); isOpaque = true; background = java.awt.Color(12,17,24) }
    init { check(SwingUtilities.isEventDispatchThread()) }
    fun update(value: EditorState) { check(SwingUtilities.isEventDispatchThread()); current = value; ensurePanel() }
    private fun ensurePanel() {
        if (stopped || panel != null || frame == null) return
        val transform = container.graphicsConfiguration?.defaultTransform
        if (!giantSurfaceAdmitted(container.width, container.height, transform?.scaleX ?: 1.0, transform?.scaleY ?: 1.0)) return
        val created = ComposePanel().apply {
            isDisposeOnRemove = true
            setContent {
                MaterialTheme {
                    Box(Modifier.fillMaxSize().onPreviewKeyEvent(shortcutRoute::dispatch)) {
                        EditorCanvas(controller, current, giantFrame = frame)
                        if (current.canvasReady) {
                            EraserOverlay(controller.erasers,current.view.camera,current.document?.render?.revision,current.document?.documentId)
                            SelectionOverlay(controller.selections,current.view.camera,current.document?.render?.revision,current.document?.documentId)
                        }
                        if (!current.canvasReady || current.renderIssue != null) Text(current.renderIssue ?: "Preparing giant-image tilesÃ¢â‚¬Â¦", Modifier.padding(24.dp))
                    }
                }
            }
        }
        panel = created; container.add(created); container.revalidate(); container.repaint()
    }
    override suspend fun create(pixels: GiantTilePixels): GiantBitmap {
        currentCoroutineContext().ensureActive()
        // One <=256Ã‚Â² raster copy, no encoded intermediates or filesystem I/O.
        val image = Image.makeRaster(ImageInfo(pixels.width.toInt(), pixels.height.toInt(), ColorType.RGBA_8888,
            ColorAlphaType.OPAQUE, ColorSpace.sRGB), pixels.rgba, pixels.width.toInt()*4)
        try { return GiantBitmap(image.toComposeImageBitmap(), image) }
        catch (failure: Throwable) { image.close(); throw failure }
    }
    override fun present(frame: GiantVisibleFrame<GiantBitmap>?) {
        check(SwingUtilities.isEventDispatchThread()); this.frame = frame; ensurePanel()
    }
    private fun retireNow() {
        check(SwingUtilities.isEventDispatchThread())
        val old = panel ?: return
        controller.cancelInput()
        // Removing invokes removeNotify; explicit dispose also covers a panel
        // that never became displayable. The detached public API is the fence.
        container.remove(old); old.dispose(); panel = null
        container.revalidate(); container.repaint()
    }
    override suspend fun retire() { withContext(NonCancellable + Dispatchers.Main.immediate) { retireNow() } }
    override fun release(image: GiantBitmap) { image.release() }
    fun stop() { stopped = true; shortcutRoute.seal() }
}

internal class GiantDesktopView private constructor(
    private val controller: EditorController, private val image: GiantImage, val host: GiantCanvasHost,
    private val session: GiantViewportSession<GiantBitmap>, private val observation: Job,
) {
    companion object {
        fun create(scope: CoroutineScope, controller: EditorController, image: GiantImage, state: EditorState): GiantDesktopView {
            val host = GiantCanvasHost(controller, state)
            val session = GiantViewportSession(scope, image, host)
            val observe = scope.launch { session.state.collect { value ->
                if (value.message != null) controller.report(value.message)
                controller.giantReady(value.ready)
            } }
            return GiantDesktopView(controller, image, host, session, observe)
        }
    }
    fun update(state: EditorState) {
        host.update(state)
        val camera = state.view.camera
        if (camera.viewportWidth <= 0 || camera.viewportHeight <= 0 || camera.viewportWidth*camera.viewportHeight > 16.0*1024*1024) return
        val points = controller.core.mapPoints(camera,true,listOf(Point(0.0,0.0),Point(camera.viewportWidth,0.0),Point(camera.viewportWidth,camera.viewportHeight),Point(0.0,camera.viewportHeight)))
        val matrix = controller.core.cameraMatrix(camera)
        session.request(GiantViewport(points.minOf{it.x},points.minOf{it.y},points.maxOf{it.x},points.maxOf{it.y},matrix.a,matrix.b,matrix.c,matrix.d))
    }
    suspend fun close() { host.stop(); withContext(NonCancellable) { observation.cancelAndJoin(); session.close() } }
    suspend fun export(options: GiantPngOptions): GiantPngReceipt = session.withExclusiveImage { it.exportPng(options) }
}

@Composable internal fun GiantEditorCanvas(view: GiantDesktopView, state: EditorState, onShortcut:(KeyEvent)->Boolean) {
    SwingPanel(factory = { view.host.container }, update = { view.host.shortcuts(onShortcut);view.host.update(state) }, modifier = Modifier.fillMaxSize())
}

internal fun DrawScope.drawGiantTiles(frame: GiantVisibleFrame<GiantBitmap>) {
    for (tile in frame.tiles) {
        val p = tile.pixels
        // Keep D positions in the transform; src/dst Int overload performs no
        // rounding of the viewport/camera and has the pinned filterQuality API.
        withTransform({ translate(p.x.toFloat(), p.y.toFloat()); scale(p.documentWidth.toFloat()/p.width.toFloat(), p.documentHeight.toFloat()/p.height.toFloat(), androidx.compose.ui.geometry.Offset.Zero) }) {
            drawImage(tile.image.image, srcOffset=IntOffset.Zero, srcSize=IntSize(p.width.toInt(),p.height.toInt()),
                dstOffset=IntOffset.Zero,dstSize=IntSize(p.width.toInt(),p.height.toInt()),filterQuality=FilterQuality.None)
        }
    }
}

internal const val GIANT_EXPORT_BYTES: Long = 4L * 1024L * 1024L * 1024L
internal fun admitGiantExport(request: ExportRequest) {
    val s = request.settings
    if (s.encoding != ExportEncoding.Png8 || s.matte != ExportMatte.Preserve || s.convertToSrgb || s.allowDepthReduction || !s.assumeUntaggedSrgb)
        throw HandoffFailure("Giant export currently requires PNG8, preserved alpha/depth and the explicit untagged-sRGB assumption. No resize or conversion is implied.")
    if (request.width == 0u || request.height == 0u || request.width.toULong()*request.height.toULong() > 1_000_000_000uL)
        throw HandoffFailure("The giant output region is outside the admitted source extent.")
}
