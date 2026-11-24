package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.BlendMode
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ColorSpace
import android.graphics.HardwareRenderer
import android.graphics.PixelFormat
import android.graphics.RenderNode
import android.hardware.HardwareBuffer
import android.media.Image
import android.media.ImageReader
import android.os.Build
import android.util.Log
import java.time.Duration

/** Draws pictures the graphics processor keeps. Each [render] plays drawing
 * commands into a buffer of [width] x [height] pixels off screen, with the
 * same renderer that draws the canvas, and hands the buffer back as a bitmap
 * that any hardware canvas can draw without copying it.
 *
 * One code path for every Android version the app supports: a platform
 * [HardwareRenderer] drawing into an [ImageReader]. A render waits until the
 * picture is finished, so nothing here is asynchronous and no thread is made.
 * A renderer that has just drawn may skip the next frame (see [render]); the
 * render is then made again once.
 *
 * A picture is never drawn into again while it is held. Its buffer goes back
 * to be reused only at [Picture.release]; the caller releases a picture once
 * no frame that draws it can still be waiting. [close] ends drawing but leaves
 * every picture already made readable, so it is safe at any time.
 *
 * Used from one thread at a time. */
internal class PictureSurface(val width: Int, val height: Int) {
    /** One finished picture. [bitmap] shows the buffer; premultiplied sRGB
     * with alpha, transparent wherever nothing was drawn. */
    class Picture internal constructor(val bitmap: Bitmap, private val image: Image, private val owner: PictureSurface) {
        private var released = false
        /** Gives the buffer back. The bitmap must not be drawn afterwards. */
        fun release() {
            if (released) return
            released = true
            bitmap.recycle(); owner.returned(image)
        }
    }

    private val reader = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, MAX_PICTURES,
        HardwareBuffer.USAGE_GPU_SAMPLED_IMAGE or HardwareBuffer.USAGE_GPU_COLOR_OUTPUT)
    // Not `apply`: inside it, width and height would be the node's own, still zero.
    private val node = RenderNode("vw-kept-picture").also { it.setPosition(0, 0, width, height) }
    private val renderer = HardwareRenderer()
    private var out = 0
    @Volatile var closed = false
        private set

    init {
        try {
            // Not opaque: the picture keeps alpha, and is cleared before each render.
            renderer.isOpaque = false
            renderer.setContentRoot(node)
            renderer.setSurface(reader.surface)
            renderer.start()
        } catch (error: Throwable) { close(); throw error }
    }

    /** Whether another picture can be held besides those not yet released. */
    val room: Boolean get() = !closed && out < MAX_PICTURES

    /** Draws one picture. Null when it could not be drawn; the surface is then
     * closed. Pictures made before stay good. */
    fun render(draw: (Canvas) -> Unit): Picture? {
        if (!room) return null
        try {
            var image = frame(draw)
            if (image == null) {
                // The renderer puts off or skips a frame it takes to be a second
                // one within the same display frame: within two milliseconds of
                // its last (and then says so), and on recent versions, without a
                // word, until another window of the app has drawn. A picture is
                // wanted now. Setting the surface again starts the renderer's
                // frame history afresh, and its first frame is always drawn.
                renderer.setSurface(reader.surface)
                retried++
                image = frame(draw)
            }
            if (image == null) return failed("no picture came out of the renderer")
            out++
            val bitmap = try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && !finished(image)) null
                else image.hardwareBuffer?.let { buffer -> try { Bitmap.wrapHardwareBuffer(buffer, SRGB) } finally { buffer.close() } }
            } catch (error: RuntimeException) { null }
            if (bitmap == null) { returned(image); return failed("the picture was not finished or could not be read") }
            return Picture(bitmap, image, this)
        } catch (error: RuntimeException) { return failed(error.javaClass.simpleName) }
    }

    /** Records [draw], has the renderer draw it and takes the result; null
     * when nothing came out. */
    private fun frame(draw: (Canvas) -> Unit): Image? {
        val canvas = node.beginRecording()
        try { canvas.drawColor(Color.TRANSPARENT, BlendMode.CLEAR); draw(canvas) } finally { node.endRecording() }
        val result = renderer.createRenderRequest().setWaitForPresent(true).syncAndDraw()
        // The commands hold the picture drawn before and the document's paths.
        node.discardDisplayList()
        if (result and NOT_DRAWN != 0) return null
        // The newest, should a frame the renderer put off have come out since.
        return reader.acquireLatestImage()
    }

    /** On-device tests only: pictures that came out only once the surface was set again. */
    var retried = 0
        private set

    /** Fixed words and numbers only; the canvas falls back to drawing whole scenes. */
    private fun failed(why: String): Picture? {
        Log.w(TAG, "kept picture not drawn: $why")
        close()
        return null
    }

    /** Waits for the graphics processor to finish the picture, where the
     * platform says when that is. Earlier versions have no way to ask; there
     * the picture is only ever read by the renderer that drew it, in order. */
    @androidx.annotation.RequiresApi(Build.VERSION_CODES.TIRAMISU)
    private fun finished(image: Image): Boolean = try {
        image.fence.let { fence -> try { fence.await(Duration.ofMillis(FENCE_WAIT_MS)) } finally { fence.close() } }
    } catch (error: java.io.IOException) { false }

    private fun returned(image: Image) {
        out--
        try { image.close() } catch (error: RuntimeException) { /* The reader is closed; its buffers went with it. */ }
    }

    /** Ends drawing and frees every buffer no picture still shows. */
    fun close() {
        if (closed) return
        closed = true
        try { renderer.stop(); renderer.destroy() } catch (error: RuntimeException) { /* Already gone. */ }
        try { reader.close() } catch (error: RuntimeException) { /* Already gone. */ }
        node.discardDisplayList()
    }

    private companion object {
        const val TAG = "VW_CANVAS"
        /** The picture shown, up to two waiting to be released, and the one being drawn. */
        const val MAX_PICTURES = 4
        const val FENCE_WAIT_MS = 500L
        const val NOT_DRAWN = HardwareRenderer.SYNC_LOST_SURFACE_REWARD_IF_FOUND or HardwareRenderer.SYNC_CONTEXT_IS_STOPPED or
            HardwareRenderer.SYNC_FRAME_DROPPED
        val SRGB: ColorSpace = ColorSpace.get(ColorSpace.Named.SRGB)
    }
}
