package com.visualworkbench.android.editor

import android.graphics.Bitmap
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import com.visualworkbench.shared.AiRegion
import com.visualworkbench.shared.AiPixels
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicReference

/** Never recycle a published bitmap while Android's render thread may hold it. */
internal class AiBitmap(val image: Bitmap) {
    val display: ImageBitmap = image.asImageBitmap()
    fun abandon() { image.recycle() }
}

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

/** The native caller supplies straight sRGB RGBA8, including its exact region.
 * Once returned this immutable-by-ownership Bitmap can be referenced by a Canvas
 * scene; do not recycle it while a render thread may still hold that scene. */
internal suspend fun aiBitmap(region: AiRegion, width: UInt, height: UInt, rgba: ByteArray): AiBitmap {
    aiCheckPixels(region, width, height, rgba)
    val context = currentCoroutineContext()
    context.ensureActive()
    val w = region.width.toInt()
    val h = region.height.toInt()
    val image = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
    try {
        val row = IntArray(w)
        for (y in 0 until h) {
            context.ensureActive()
            for (x in 0 until w) row[x] = aiArgb(rgba, (y * w + x) * 4)
            image.setPixels(row, 0, w, 0, y, w, 1)
        }
        context.ensureActive()
        return AiBitmap(image)
    } catch (failure: Throwable) {
        // Unpublished image: no Canvas/RenderThread has acquired it.
        image.recycle()
        throw failure
    }
}
