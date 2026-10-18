package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.ColorSpace
import com.visualworkbench.shared.GiantTilePixels
import com.visualworkbench.shared.GiantViewportSession
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

private const val MAX_TILE_RGBA = 256L * 256L * 4L

/** Incoming ByteArray, actual CPU allocation, one bounded texture, temporary row
 * and bookkeeping all fit the already reserved TILE_CHARGE. Driver overhead is
 * not measured by this check and is not claimed as whole-process admission. */
internal fun admitGiantAndroidBitmap(width: Int, height: Int, inputBytes: Int,
    rowBytes: Int, allocationBytes: Int) {
    require(width in 1..256 && height in 1..256)
    require(inputBytes.toLong() == width.toLong() * height * 4L)
    require(rowBytes.toLong() >= width * 4L && allocationBytes.toLong() >= rowBytes.toLong() * height)
    require(allocationBytes.toLong() <= MAX_TILE_RGBA + 4096L)
    val known = inputBytes.toLong() + allocationBytes + MAX_TILE_RGBA + 4096L + width * 4L + 4096L
    require(known <= GiantViewportSession.TILE_CHARGE)
}

/** Keeps an acquired object across prompt dispatcher cancellation. Failed
 * creations stay owned until the caller has joined real renderer retirement.
 * This is one backend's private pending storage, not a renderer registry. */
internal class GiantAndroidBitmapHandoff<T : Any> {
    private val failed = mutableListOf<T>()
    internal val retainedCount: Int get() = failed.size

    suspend fun create(dispatcher: CoroutineDispatcher, convert: suspend ((T) -> Unit) -> T): T {
        val pending = AtomicReference<T?>(null)
        try {
            val value = withContext(dispatcher) {
                convert { acquired -> check(pending.compareAndSet(null, acquired)) }
            }
            currentCoroutineContext().ensureActive()
            check(pending.compareAndSet(value, null))
            return value // No suspension between clearing pending and caller adoption.
        } finally {
            pending.getAndSet(null)?.let { failed += it }
        }
    }

    /** Call only AFTER the exact pipeline release callback has joined. A failed
     * release retains that object and the remaining pending objects for retry. */
    fun releaseRetired(release: (T) -> Unit) {
        val iterator = failed.iterator()
        while (iterator.hasNext()) { release(iterator.next()); iterator.remove() }
    }
}

internal class GiantAndroidBitmapFactory {
    private val handoff = GiantAndroidBitmapHandoff<Bitmap>()
    suspend fun create(pixels: GiantTilePixels): Bitmap = handoff.create(Dispatchers.Default) { acquired ->
        require(pixels.width in 1u..256u && pixels.height in 1u..256u)
        val width = pixels.width.toInt(); val height = pixels.height.toInt()
        require(pixels.rgba.size == width * height * 4)
        require((3 until pixels.rgba.size step 4).all { pixels.rgba[it] == (-1).toByte() })
        currentCoroutineContext().ensureActive()
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888,
            false, ColorSpace.get(ColorSpace.Named.SRGB))
        acquired(bitmap)
        admitGiantAndroidBitmap(width, height, pixels.rgba.size, bitmap.rowBytes, bitmap.allocationByteCount)
        check(bitmap.config == Bitmap.Config.ARGB_8888 && !bitmap.hasAlpha() && bitmap.colorSpace?.isSrgb == true)
        val row = IntArray(width)
        for (y in 0 until height) {
            currentCoroutineContext().ensureActive()
            for (x in row.indices) {
                val i = (y * width + x) * 4
                row[x] = (0xff shl 24) or ((pixels.rgba[i].toInt() and 255) shl 16) or
                    ((pixels.rgba[i + 1].toInt() and 255) shl 8) or (pixels.rgba[i + 2].toInt() and 255)
            }
            bitmap.setPixels(row, 0, width, 0, y, width, 1)
        }
        // No prepareToDraw/upload or graphics reader exists before admission.
        bitmap
    }
    fun releaseFailedAfterRetirement(): Unit = handoff.releaseRetired(::releaseGiantAndroidBitmap)
}

internal fun releaseGiantAndroidBitmap(bitmap: Bitmap) {
    if (!bitmap.isRecycled) bitmap.recycle()
}
