package com.visualworkbench.android.stream

import android.graphics.Bitmap
import com.visualworkbench.shared.CaptureStreamPixels
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

/** Resource allocation and delivery stay on the caller dispatcher; no native
 * or Bitmap owner is returned across a cancellable dispatcher switch. */
internal suspend fun readStreamBitmap(pixels: CaptureStreamPixels): Bitmap {
    val frame = pixels.frame
    StreamCanvasView.admitBitmap(frame.width.toLong(), frame.height.toLong())
    val bitmap = Bitmap.createBitmap(frame.width.toInt(), frame.height.toInt(), Bitmap.Config.ARGB_8888)
    try {
        val row = IntArray(bitmap.width)
        for (y in 0 until bitmap.height) {
            currentCoroutineContext().ensureActive()
            val rgba = pixels.rgbaRows(y.toUInt(), 1u)
            require(rgba.size == row.size * 4)
            for (x in row.indices) { val p=x*4; row[x] = ((rgba[p+3].toInt() and 255) shl 24) or
                ((rgba[p].toInt() and 255) shl 16) or ((rgba[p+1].toInt() and 255) shl 8) or (rgba[p+2].toInt() and 255) }
            bitmap.setPixels(row, 0, bitmap.width, 0, y, bitmap.width, 1)
        }
        return bitmap
    } catch (error: Throwable) { bitmap.recycle(); throw error }
}
