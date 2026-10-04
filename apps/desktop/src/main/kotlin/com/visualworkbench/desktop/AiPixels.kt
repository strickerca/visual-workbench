package com.visualworkbench.desktop

import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import com.visualworkbench.shared.AiRegion
import com.visualworkbench.shared.AiPixels
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import org.jetbrains.skia.Image
import java.awt.image.BufferedImage
import java.io.OutputStream
import javax.imageio.ImageIO
import javax.imageio.stream.MemoryCacheImageOutputStream
import java.util.concurrent.atomic.AtomicReference

/** Only unpublished images may be abandoned explicitly. Once a frame is
 * published, Compose/Skia may still retain it after the controller replaces the
 * frame; its native wrapper then follows the renderer's ordinary GC lifetime. */
internal class AiBitmap(val image: ImageBitmap, private val native: Image) {
    val display: ImageBitmap get() = image
    fun abandon() { native.close() }
}

/** The handoff cell is populated on the worker before the dispatcher boundary:
 * prompt cancellation cannot lose a newly allocated native image. */
internal suspend fun aiReadBitmap(value: AiPixels): AiBitmap {
    val handoff = AtomicReference<AiBitmap?>(null)
    try {
        val bitmap = withContext(Dispatchers.Default) {
            aiBitmap(value.region, value.canvasWidth, value.canvasHeight, value.rgbaSrgb).also(handoff::set)
        }
        currentCoroutineContext().ensureActive()
        handoff.set(null)
        return bitmap
    } finally { handoff.getAndSet(null)?.abandon() }
}

/** Existing desktop codec path, constrained to a single <=512px tile. Encoding
 * stays in memory with a fixed cap; ImageIO's global disk-cache setting is never
 * changed. The returned ImageBitmap follows Compose/Skia render ownership. */
internal suspend fun aiBitmap(region: AiRegion, width: UInt, height: UInt, rgba: ByteArray): AiBitmap {
    aiCheckPixels(region, width, height, rgba)
    val context = currentCoroutineContext()
    context.ensureActive()
    val w = region.width.toInt()
    val h = region.height.toInt()
    val image = BufferedImage(w, h, BufferedImage.TYPE_INT_ARGB)
    val row = IntArray(w)
    for (y in 0 until h) {
        context.ensureActive()
        for (x in 0 until w) row[x] = aiArgb(rgba, (y * w + x) * 4)
        image.setRGB(0, y, w, 1, row, 0, w)
    }
    val encoded = AiPngBuffer()
    MemoryCacheImageOutputStream(encoded).use { target ->
        if (!ImageIO.write(image, "png", target)) throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
    }
    context.ensureActive()
    val native = Image.makeFromEncoded(encoded.bytes())
    try {
        context.ensureActive()
        return AiBitmap(native.toComposeImageBitmap(), native)
    } catch (failure: Throwable) {
        // This Skia image has not been handed to Compose.
        native.close()
        throw failure
    }
}

private class AiPngBuffer : OutputStream() {
    private val buffer = ByteArray(2 * 1024 * 1024)
    private var size = 0
    override fun write(value: Int) {
        if (size == buffer.size) throw AiDisplayRefusal(AiDisplayRefusal.Reason.ZoomIn)
        buffer[size++] = value.toByte()
    }
    override fun write(bytes: ByteArray, offset: Int, length: Int) {
        if (offset < 0 || length < 0 || offset > bytes.size - length || length > buffer.size - size)
            throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
        bytes.copyInto(buffer, size, offset, offset + length)
        size += length
    }
    fun bytes(): ByteArray = buffer.copyOf(size)
}
