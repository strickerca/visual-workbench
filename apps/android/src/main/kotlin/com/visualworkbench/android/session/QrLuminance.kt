package com.visualworkbench.android.session

import java.nio.ByteBuffer

internal data class QrLuminance(val bytes: ByteArray, val width: Int, val height: Int)

/** Camera negotiation may exceed the requested resolution. Sample that frame
 * instead of discarding every image; allocation and sampled work stay bounded.
 * The plane's current position is its origin, and row padding/pixel stride are
 * independent of the crop. No camera bytes are stored or logged. */
internal fun sampleQrLuminance(
    plane: ByteBuffer, rowStride: Int, pixelStride: Int,
    imageWidth: Int, imageHeight: Int,
    left: Int, top: Int, width: Int, height: Int,
): QrLuminance? {
    if (imageWidth !in 1..16_384 || imageHeight !in 1..16_384 ||
        rowStride <= 0 || pixelStride <= 0 || left < 0 || top < 0 ||
        width <= 0 || height <= 0 || left.toLong() + width > imageWidth ||
        top.toLong() + height > imageHeight ||
        (imageWidth - 1L) * pixelStride + 1 > rowStride) return null
    val buffer = plane.duplicate()
    val origin = buffer.position().toLong()
    val last = origin + (top + height - 1L) * rowStride + (left + width - 1L) * pixelStride
    if (last >= buffer.limit()) return null
    var step = 1
    fun sampled(size: Int): Int = (size + step - 1) / step
    while (sampled(width).toLong() * sampled(height) > 2_000_000) step++
    val outputWidth = sampled(width)
    val outputHeight = sampled(height)
    val gray = ByteArray(outputWidth * outputHeight)
    try {
        for (y in 0 until outputHeight) {
            val sourceY = top + (y * step + step / 2).coerceAtMost(height - 1)
            val row = origin + sourceY.toLong() * rowStride
            for (x in 0 until outputWidth) {
                val sourceX = left + (x * step + step / 2).coerceAtMost(width - 1)
                gray[y * outputWidth + x] = buffer.get((row + sourceX.toLong() * pixelStride).toInt())
            }
        }
        return QrLuminance(gray, outputWidth, outputHeight)
    } catch (_: Exception) {
        gray.fill(0)
        return null
    }
}
