package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import java.util.concurrent.atomic.AtomicReference

internal const val GIANT_ANDROID_TILE_BYTES = 96L * 1024L * 1024L

/** The independent surface allowance includes four RGBA buffers. It bounds
 * known surfaces, not driver/RSS/Canvas saveLayer internals. */
internal fun admitGiantAndroidSurface(width: Int, height: Int) {
    require(width in 1..8192 && height in 1..8192 && width.toLong() * height <= 6L * 1024L * 1024L) {
        "This physical viewport exceeds the giant-image surface allowance. Resize the app before displaying it."
    }
}

internal fun giantAndroidViewport(core: WorkbenchCore, camera: Camera): GiantViewport {
    require(camera.viewportWidth.isFinite() && camera.viewportHeight.isFinite())
    admitGiantAndroidSurface(kotlin.math.ceil(camera.viewportWidth).toInt(), kotlin.math.ceil(camera.viewportHeight).toInt())
    val corners = core.mapPoints(camera, true, listOf(Point(0.0, 0.0), Point(camera.viewportWidth, 0.0),
        Point(camera.viewportWidth, camera.viewportHeight), Point(0.0, camera.viewportHeight)))
    require(corners.size == 4 && corners.all { it.x.isFinite() && it.y.isFinite() })
    val transform = core.cameraMatrix(camera)
    return GiantViewport(corners.minOf { it.x }, corners.minOf { it.y }, corners.maxOf { it.x }, corners.maxOf { it.y },
        transform.a, transform.b, transform.c, transform.d)
}

/** Converts at most one 256px tile. The pending owner crosses the dispatcher
 * handoff so prompt cancellation cannot discard an unrecycled bitmap. */
internal suspend fun giantAndroidBitmap(pixels: GiantTilePixels): Bitmap {
    require(pixels.width in 1u..256u && pixels.height in 1u..256u)
    val width = pixels.width.toInt(); val height = pixels.height.toInt()
    require(pixels.rgba.size == width * height * 4)
    val pending = AtomicReference<Bitmap?>(null)
    try {
        val value = withContext(Dispatchers.Default) {
            val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
            pending.set(bitmap)
            require(bitmap.allocationByteCount <= 256 * 256 * 4 + 4096)
            val row = IntArray(width)
            for (y in 0 until height) {
                ensureActive()
                for (x in row.indices) {
                    val i = (y * width + x) * 4
                    require(pixels.rgba[i + 3] == (-1).toByte())
                    row[x] = (0xff shl 24) or ((pixels.rgba[i].toInt() and 255) shl 16) or
                        ((pixels.rgba[i + 1].toInt() and 255) shl 8) or (pixels.rgba[i + 2].toInt() and 255)
                }
                bitmap.setPixels(row, 0, width, 0, y, width, 1)
            }
            bitmap
        }
        currentCoroutineContext().ensureActive()
        check(pending.compareAndSet(value, null))
        return value
    } finally { pending.getAndSet(null)?.recycle() }
}

/** Main-confined owner; this MUST settle before its WorkbenchProject closes. */
internal class GiantAndroidCanvas(
    scope: CoroutineScope,
    image: GiantImage,
    private val leases: GiantRenderLeases,
    private val changed: () -> Unit,
) {
    var frame: GiantVisibleFrame<Bitmap>? = null; private set
    var status = GiantViewStatus(); private set
    private var closed = false
    private val backend = object : GiantRenderBackend<Bitmap> {
        override suspend fun create(pixels: GiantTilePixels): Bitmap = giantAndroidBitmap(pixels)
        override fun present(frame: GiantVisibleFrame<Bitmap>?) { this@GiantAndroidCanvas.frame = frame; changed() }
        override suspend fun retire() { leases.retireAll() }
        override fun release(image: Bitmap) { image.recycle() }
    }
    private val viewport = GiantViewportSession(scope, image, backend, GIANT_ANDROID_TILE_BYTES)
    private val observer = scope.launch(start = CoroutineStart.UNDISPATCHED) {
        viewport.state.collect { next ->
            val visualChange = next.ready != status.ready || next.message != status.message
            status = next
            if (visualChange && !closed) changed()
        }
    }
    fun update(core: WorkbenchCore, camera: Camera) {
        if (!closed && camera.viewportWidth > 1 && camera.viewportHeight > 1) request(giantAndroidViewport(core, camera))
    }
    internal fun request(view: GiantViewport) { if (!closed) viewport.request(view) }
    fun retry() { if (!closed) viewport.retry() }
    suspend fun exportPng(options: GiantPngOptions): GiantPngReceipt {
        check(!closed)
        val paused = leases.pauseAdmission()
        try { return viewport.withRetiredImage { it.exportPng(options) } }
        finally { paused.close() }
    }
    suspend fun close() {
        closed = true
        withContext(NonCancellable) {
            observer.cancelAndJoin()
            viewport.close()
        }
    }
}

internal fun drawGiantAndroidTiles(canvas: Canvas, frame: GiantVisibleFrame<Bitmap>) {
    val paint = Paint() // No interpolation across independent cache-tile edges.
    for (tile in frame.tiles) {
        val p = tile.pixels
        require(!tile.image.isRecycled && tile.image.width == p.width.toInt() && tile.image.height == p.height.toInt())
        canvas.drawBitmap(tile.image, Rect(0, 0, tile.image.width, tile.image.height),
            RectF(p.x.toFloat(), p.y.toFloat(), (p.x.toULong() + p.documentWidth.toULong()).toFloat(),
                (p.y.toULong() + p.documentHeight.toULong()).toFloat()), paint)
    }
}
