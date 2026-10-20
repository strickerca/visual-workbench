package com.visualworkbench.android.stream

import android.graphics.Bitmap
import com.visualworkbench.shared.*
import kotlinx.coroutines.*

internal const val STREAM_ROW_BYTES = 256 * 1024
internal const val STREAM_MAX_BITMAP_BYTES = 134_217_728uL

internal fun streamBitmapReservation(frame: CaptureStreamFrame): ULong {
    require(frame.width in 1u..65_536u && frame.height in 1u..65_536u)
    val bytes = frame.width.toULong() * frame.height * 4uL
    require(bytes <= STREAM_MAX_BITMAP_BYTES)
    // Full bitmap (including bounded stride padding), one RGBA batch and one
    // IntArray batch. This is in addition to the actual Surface buffer charge.
    return (frame.width.toULong() * 4uL + 64uL) * frame.height + 2uL * STREAM_ROW_BYTES.toULong()
}

internal class AndroidStreamPixelFactory(private val resources: SessionResources) : StreamPixelFactory<Bitmap> {
    private val retained = linkedSetOf<Pixels>()
    private var cleanupFailed = false

    override suspend fun copy(source: CaptureStreamPixels): StreamOwnedPixels<Bitmap> {
        currentCoroutineContext().ensureActive()
        check(!cleanupFailed) { "Stream pixel retirement remains owned" }
        val owner = Pixels(source.frame)
        retained += owner // Publish cleanup ownership before reserve/dispatcher suspension.
        try {
            withContext(NonCancellable) { owner.charge = resources.reserveForeign(streamBitmapReservation(source.frame)) }
            currentCoroutineContext().ensureActive()
            withContext(Dispatchers.Default) {
                val frame = source.frame
                val width = frame.width.toInt(); val height = frame.height.toInt()
                owner.bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
                val bitmap = checkNotNull(owner.bitmap)
                require(bitmap.allocationByteCount.toULong() <= (frame.width.toULong() * 4uL + 64uL) * frame.height)
                bitmap.setHasAlpha(false)
                val batchRows = maxOf(1, STREAM_ROW_BYTES / (width * 4))
                val argb = IntArray(width * minOf(batchRows, height))
                var first = 0
                while (first < height) {
                    ensureActive()
                    val rows = minOf(batchRows, height - first)
                    val bytes = source.rgbaRows(first.toUInt(), rows.toUInt())
                    require(bytes.size == width * rows * 4 && bytes.size <= STREAM_ROW_BYTES)
                    for (pixel in 0 until width * rows) {
                        val at = pixel * 4
                        require((bytes[at + 3].toInt() and 255) == 255) { "Stream pixels must be opaque" }
                        argb[pixel] = (255 shl 24) or ((bytes[at].toInt() and 255) shl 16) or
                            ((bytes[at + 1].toInt() and 255) shl 8) or (bytes[at + 2].toInt() and 255)
                    }
                    bitmap.setPixels(argb, 0, width, 0, first, width, rows)
                    first += rows
                }
            }
            currentCoroutineContext().ensureActive()
            return owner
        } catch (error: Throwable) {
            withContext(NonCancellable) {
                try { owner.close() } catch (cleanup: Throwable) { if (cleanup !== error) error.addSuppressed(cleanup) }
            }
            throw error
        }
    }

    override suspend fun close() { for (owner in retained.toList()) owner.close(); cleanupFailed = false }

    private inner class Pixels(override val frame: CaptureStreamFrame) : StreamOwnedPixels<Bitmap> {
        var bitmap: Bitmap? = null
        var charge: SessionResourceCharge? = null
        override val value: Bitmap get() = checkNotNull(bitmap)
        override suspend fun close() {
            // Software Surface Canvas copies pixels synchronously. The lifecycle
            // calls this only after post or actual surface retirement settles.
            bitmap?.let { if (!it.isRecycled) it.recycle(); bitmap = null }
            charge?.let {
                try { it.close(); charge = null } catch (error: Throwable) { cleanupFailed = true; throw error }
            }
            retained.remove(this)
        }
    }
}
