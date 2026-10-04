package com.visualworkbench.android.editor

import android.graphics.Bitmap
import com.visualworkbench.shared.BackgroundImage
import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicReference

/** No UI publication or source mutation. The CPU producer settles before an
 * undelivered bitmap is retired. Published bitmaps remain renderer-owned. */
internal suspend fun captureDisplayBitmap(image: BackgroundImage): Bitmap {
    val owned = AtomicReference<Bitmap?>(null)
    try {
        val result = withContext(Dispatchers.Default) {
            require(image.width in 1u..32768u && image.height in 1u..32768u)
            val width = image.width.toInt(); val height = image.height.toInt()
            require(width.toLong() * height <= 50_000_000L && width.toLong() * height * 4 == image.rgba.size.toLong())
            require(image.rgba.size <= 32 * 1024 * 1024) // 32 MiB additional display bitmap; no downscale.
            val out = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
            owned.set(out)
            val row = IntArray(width)
            for (y in 0 until height) {
                ensureActive()
                for (x in row.indices) {
                    val i = (y * width + x) * 4
                    row[x] = ((image.rgba[i + 3].toInt() and 255) shl 24) or ((image.rgba[i].toInt() and 255) shl 16) or
                        ((image.rgba[i + 1].toInt() and 255) shl 8) or (image.rgba[i + 2].toInt() and 255)
                }
                out.setPixels(row, 0, width, 0, y, width, 1)
            }
            out
        }
        currentCoroutineContext().ensureActive()
        owned.set(null)
        return result
    } finally { owned.getAndSet(null)?.recycle() }
}