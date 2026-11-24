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
import java.util.concurrent.atomic.AtomicInteger

/** Draws pictures the graphics processor keeps. Each [render] plays drawing
 * commands into a buffer of [width] x [height] pixels off screen, with the
 * same renderer that draws the canvas, and hands the buffer back as a bitmap
 * that any hardware canvas can draw without copying it.
 *
 * One code path for every Android version the app supports: a platform
 * [HardwareRenderer] drawing into an [ImageReader] (a [Target]). A render
 * waits until the picture is finished, so nothing here is asynchronous and no
 * thread is made.
 *
 * Every picture handed back is the one just drawn. The platform renderer may
 * skip a frame it takes for a second one within a display frame: within two
 * milliseconds of its last it puts it off and says so, and on recent versions
 * it skips it without a word. Either way it keeps the frame back and draws it
 * by itself at a later display frame. So:
 *
 * - A target whose render did not come out is never drawn with again. It is
 *   closed, which ends its renderer and whatever it kept back, and the render
 *   is made once more on a new target, whose first frame is always drawn.
 * - A target that is used again has had every frame it was asked for come
 *   out, so it keeps nothing back. Before each render every image waiting in
 *   its reader is dropped; after it, the newest image is taken. Only this
 *   render's commands can have been drawn in between: the renderer draws
 *   nothing unasked, and a frame it drew late draws the commands last given
 *   to it, which are this render's.
 * - The reader has room for every picture that may be held and one more, so
 *   taking the newest image never stops short at its limit.
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
    class Picture internal constructor(val bitmap: Bitmap, private val image: Image, private val owner: Target) {
        private var released = false
        /** Gives the buffer back. The bitmap must not be drawn afterwards. */
        fun release() {
            if (released) return
            released = true
            livePictures.decrementAndGet()
            bitmap.recycle(); owner.returned(image)
        }
        init { livePictures.incrementAndGet() }
    }

    /** One platform renderer drawing into one reader. */
    internal class Target(width: Int, height: Int) {
        private val reader = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, MAX_HELD + 1,
            HardwareBuffer.USAGE_GPU_SAMPLED_IMAGE or HardwareBuffer.USAGE_GPU_COLOR_OUTPUT)
        // Not `apply`: inside it, width and height would be the node's own, still zero.
        private val node = RenderNode("vw-kept-picture").also { it.setPosition(0, 0, width, height) }
        private val renderer: HardwareRenderer = try { HardwareRenderer() } catch (error: Throwable) { reader.close(); throw error }
        /** Pictures of this target not yet released. */
        var out = 0
        var closed = false
            private set

        init {
            liveSurfaces.incrementAndGet()
            try {
                // Not opaque: the picture keeps alpha, and is cleared before each render.
                renderer.isOpaque = false
                renderer.setContentRoot(node)
                renderer.setSurface(reader.surface)
                renderer.start()
            } catch (error: Throwable) { close(); throw error }
        }

        /** Draws [draw] and takes the image of it; null when none came out. */
        fun frame(draw: (Canvas) -> Unit): Image? {
            // Nothing should be waiting; whatever is, is not this render.
            while (true) (reader.acquireNextImage() ?: break).close()
            val canvas = node.beginRecording()
            try { canvas.drawColor(Color.TRANSPARENT, BlendMode.CLEAR); draw(canvas) } finally { node.endRecording() }
            val result = renderer.createRenderRequest().setWaitForPresent(true).syncAndDraw()
            // The commands hold the picture drawn before and the document's paths.
            node.discardDisplayList()
            if (result and NOT_DRAWN != 0) return null
            // The newest, one at a time: at most one is held while the next is acquired.
            var newest: Image? = null
            while (true) {
                val next = reader.acquireNextImage() ?: break
                newest?.let { it.close(); duplicates.incrementAndGet() }
                newest = next
            }
            return newest
        }

        fun returned(image: Image) {
            out--
            try { image.close() } catch (error: RuntimeException) { /* The reader is closed; its buffers went with it. */ }
        }

        fun close() {
            if (closed) return
            closed = true
            liveSurfaces.decrementAndGet()
            try { renderer.stop(); renderer.destroy() } catch (error: RuntimeException) { /* Already gone. */ }
            try { reader.close() } catch (error: RuntimeException) { /* Already gone. */ }
            node.discardDisplayList()
        }
    }

    private var target: Target? = Target(width, height)

    /** Whether drawing has ended for good. */
    val closed: Boolean get() = target == null

    /** Whether another picture can be held besides those not yet released. */
    val room: Boolean get() = target?.let { it.out < MAX_HELD } == true

    /** On-device tests only: renders made again on a new target. */
    var retried = 0
        private set

    /** Draws one picture. Null when it could not be drawn; the surface is then
     * closed. Pictures made before stay good. */
    fun render(draw: (Canvas) -> Unit): Picture? {
        if (!room) return null
        var used = target ?: return null
        try {
            var image = used.frame(draw)
            if (image == null) {
                // Put off or skipped: the renderer may still draw it later, so
                // this target is done with. The pictures it made stay readable.
                used.close()
                retried++
                used = Target(width, height).also { target = it }
                image = used.frame(draw)
            }
            if (image == null) return failed("no picture came out of the renderer")
            used.out++
            val bitmap = try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && !finished(image)) null
                else image.hardwareBuffer?.let { buffer -> try { Bitmap.wrapHardwareBuffer(buffer, SRGB) } finally { buffer.close() } }
            } catch (error: RuntimeException) { null }
            if (bitmap == null) { used.returned(image); return failed("the picture was not finished or could not be read") }
            return Picture(bitmap, image, used)
        } catch (error: RuntimeException) { return failed(error.javaClass.simpleName) }
    }

    /** Fixed words and numbers only; the canvas falls back to drawing whole scenes. */
    private fun failed(why: String): Picture? {
        Log.w(TAG, "kept picture not drawn: $why")
        close()
        return null
    }

    /** Waits for the graphics processor to finish the picture, where the
     * platform says when that is (API 33 on). Earlier versions have no way to
     * ask; there the picture is only ever read by the renderer that drew it, in
     * order. */
    @androidx.annotation.RequiresApi(Build.VERSION_CODES.TIRAMISU)
    private fun finished(image: Image): Boolean = try {
        image.fence.let { fence -> try { fence.await(Duration.ofMillis(FENCE_WAIT_MS)) } finally { fence.close() } }
    } catch (error: java.io.IOException) { false }

    /** Ends drawing and frees every buffer no picture still shows. */
    fun close() {
        target?.close(); target = null
    }

    companion object {
        /** On-device tests only: targets not yet closed and pictures not yet
         * released in this process, and older images found behind the newest
         * after a render. */
        internal val liveSurfaces = AtomicInteger()
        internal val livePictures = AtomicInteger()
        internal val duplicates = AtomicInteger()
        private const val TAG = "VW_CANVAS"
        /** Pictures held at once: the one shown, up to two waiting to be released, and the one being drawn. */
        private const val MAX_HELD = 4
        private const val FENCE_WAIT_MS = 500L
        private const val NOT_DRAWN = HardwareRenderer.SYNC_LOST_SURFACE_REWARD_IF_FOUND or HardwareRenderer.SYNC_CONTEXT_IS_STOPPED or
            HardwareRenderer.SYNC_FRAME_DROPPED
        private val SRGB: ColorSpace = ColorSpace.get(ColorSpace.Named.SRGB)
    }
}
