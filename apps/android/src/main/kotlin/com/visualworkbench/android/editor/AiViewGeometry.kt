package com.visualworkbench.android.editor

import com.visualworkbench.shared.AiCompareMode
import com.visualworkbench.shared.AiRegion
import com.visualworkbench.shared.Camera
import com.visualworkbench.shared.Point
import kotlin.math.ceil
import kotlin.math.floor

internal const val AI_TILE_EDGE: Int = 512
internal const val AI_DISPLAY_BYTES: Long = 64L * 1024L * 1024L
internal const val AI_CONTACT_POINTS: Int = 4096

/** Display admission is separate from the native request/decode budget. A large
 * view is refused with a zoom-in message; it is never downsampled or cropped
 * silently. Framework/render-thread retention is outside this one-frame cap. */
internal class AiDisplayRefusal(val reason: Reason) : Exception(reason.name) {
    enum class Reason { Invalid, ZoomIn, ContactLimit, Stale }
}
internal data class AiTilePlan(val width: UInt, val height: UInt, val regions: List<AiRegion>, val rgbaBytes: Long)

internal fun aiCanvasWidth(sourceWidth: UInt, mode: AiCompareMode): UInt {
    if (sourceWidth == 0u || sourceWidth > Int.MAX_VALUE.toUInt())
        throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
    val width = sourceWidth.toULong() * if (mode == AiCompareMode.Split) 2uL else 1uL
    if (width > Int.MAX_VALUE.toULong()) throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
    return width.toUInt()
}

/** corners must come from WorkbenchCore.mapPoints(camera, true, ...).
 * Only integer tile clipping lives here; no second camera/affine implementation. */
internal fun aiTilePlan(
    width: UInt,
    height: UInt,
    corners: List<Point>,
    maxRgbaBytes: Long = AI_DISPLAY_BYTES,
): AiTilePlan {
    if (width == 0u || height == 0u || width > Int.MAX_VALUE.toUInt() || height > Int.MAX_VALUE.toUInt() ||
        corners.size != 4 || corners.any { !it.x.isFinite() || !it.y.isFinite() } ||
        maxRgbaBytes !in 4..AI_DISPLAY_BYTES)
        throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
    val left = floor(corners.minOf { it.x }.coerceIn(0.0, width.toDouble())).toLong()
    val top = floor(corners.minOf { it.y }.coerceIn(0.0, height.toDouble())).toLong()
    val right = ceil(corners.maxOf { it.x }.coerceIn(0.0, width.toDouble())).toLong()
    val bottom = ceil(corners.maxOf { it.y }.coerceIn(0.0, height.toDouble())).toLong()
    if (right <= left || bottom <= top) return AiTilePlan(width, height, emptyList(), 0)
    val firstX = left / AI_TILE_EDGE * AI_TILE_EDGE
    val firstY = top / AI_TILE_EDGE * AI_TILE_EDGE
    val endX = minOf(width.toLong(), ((right + AI_TILE_EDGE - 1) / AI_TILE_EDGE) * AI_TILE_EDGE)
    val endY = minOf(height.toLong(), ((bottom + AI_TILE_EDGE - 1) / AI_TILE_EDGE) * AI_TILE_EDGE)
    val columns = (endX - firstX + AI_TILE_EDGE - 1) / AI_TILE_EDGE
    val rows = (endY - firstY + AI_TILE_EDGE - 1) / AI_TILE_EDGE
    // Cap before multiplication/list allocation. Both differences fit Int, so
    // the admitted <=256 tiles also makes all following byte arithmetic safe.
    if (columns > 256 || rows > 256 || columns * rows > 256)
        throw AiDisplayRefusal(AiDisplayRefusal.Reason.ZoomIn)
    val bytes = (endX - firstX) * (endY - firstY) * 4
    if (bytes > maxRgbaBytes) throw AiDisplayRefusal(AiDisplayRefusal.Reason.ZoomIn)
    val tiles = ArrayList<AiRegion>((columns * rows).toInt())
    var y = firstY
    while (y < endY) {
        var x = firstX
        while (x < endX) {
            tiles += AiRegion(x.toUInt(), y.toUInt(), minOf(AI_TILE_EDGE.toLong(), endX - x).toUInt(),
                minOf(AI_TILE_EDGE.toLong(), endY - y).toUInt())
            x += AI_TILE_EDGE
        }
        y += AI_TILE_EDGE
    }
    return AiTilePlan(width, height, tiles, bytes)
}

internal fun aiCheckPixels(region: AiRegion, width: UInt, height: UInt, bytes: ByteArray) {
    if (region.width !in 1u..AI_TILE_EDGE.toUInt() || region.height !in 1u..AI_TILE_EDGE.toUInt() ||
        width == 0u || height == 0u ||
        region.x.toULong() + region.width.toULong() > width.toULong() ||
        region.y.toULong() + region.height.toULong() > height.toULong() ||
        region.width.toULong() * region.height.toULong() * 4uL != bytes.size.toULong())
        throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
}

/** Straight RGBA8 from native sRGB display conversion. This packs channels only;
 * source/profile conversion, compositing and proof stay in the native core. */
internal fun aiArgb(bytes: ByteArray, offset: Int): Int {
    if (offset < 0 || offset > bytes.size - 4) throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
    return ((bytes[offset + 3].toInt() and 255) shl 24) or
        ((bytes[offset].toInt() and 255) shl 16) or
        ((bytes[offset + 1].toInt() and 255) shl 8) or (bytes[offset + 2].toInt() and 255)
}

internal data class AiContact(
    val candidateId: String,
    val camera: Camera,
    val points: List<Point>,
    val radius: Double,
    val subtract: Boolean,
    val clearFirst: Boolean,
)

/** Exactly one bounded real contact. Candidate/camera changes cancel the entire
 * contact. In Split mode a contact cannot cross the before/after seam. */
internal class AiContactCollector {
    private data class Contact(
        val candidateId: String, val camera: Camera, val width: UInt, val height: UInt,
        val split: Boolean, val pane: Int, val radius: Double, val subtract: Boolean,
        val clearFirst: Boolean, val points: ArrayList<Point>,
    )
    private var current: Contact? = null
    val active: Boolean get() = current != null
    val points: List<Point> get() = current?.points?.toList() ?: emptyList()

    fun begin(candidateId: String, camera: Camera, width: UInt, height: UInt, split: Boolean,
              point: Point, radius: Double, subtract: Boolean, clearFirst: Boolean) {
        cancel()
        if (candidateId.isEmpty() || candidateId.length > 128 || width == 0u || height == 0u ||
            width > Int.MAX_VALUE.toUInt() || height > Int.MAX_VALUE.toUInt() ||
            !radius.isFinite() || radius !in 0.5..256.0 ||
            !camera.scale.isFinite() || camera.scale <= 0 || !camera.rotation.isFinite() ||
            !camera.center.x.isFinite() || !camera.center.y.isFinite() ||
            !camera.viewportWidth.isFinite() || !camera.viewportHeight.isFinite() ||
            camera.viewportWidth <= 0 || camera.viewportHeight <= 0)
            throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
        val pane = if (split && point.x >= width.toDouble()) 1 else 0
        val admitted = sourcePoint(point, width, height, split, pane)
        current = Contact(candidateId, camera, width, height, split, pane, radius, subtract, clearFirst,
            arrayListOf(admitted))
    }

    fun append(candidateId: String, camera: Camera, mappedPoints: List<Point>) {
        val contact = current ?: return
        try {
            if (candidateId != contact.candidateId || camera != contact.camera)
                throw AiDisplayRefusal(AiDisplayRefusal.Reason.Stale)
            // Refuse before scanning/copying an unbounded platform history list.
            if (mappedPoints.size > AI_CONTACT_POINTS ||
                contact.points.size.toLong() + mappedPoints.size > AI_CONTACT_POINTS)
                throw AiDisplayRefusal(AiDisplayRefusal.Reason.ContactLimit)
            for (point in mappedPoints) {
                val next = sourcePoint(point, contact.width, contact.height, contact.split, contact.pane)
                if (next != contact.points.last()) contact.points += next
            }
        } catch (failure: Exception) { cancel(); throw failure }
    }

    fun finish(candidateId: String, camera: Camera): AiContact? {
        val contact = current ?: return null
        current = null
        if (contact.candidateId != candidateId || contact.camera != camera)
            throw AiDisplayRefusal(AiDisplayRefusal.Reason.Stale)
        return AiContact(contact.candidateId, contact.camera, contact.points.toList(),
            contact.radius, contact.subtract, contact.clearFirst)
    }
    fun cancel() { current = null }

    private fun sourcePoint(point: Point, width: UInt, height: UInt, split: Boolean, pane: Int): Point {
        val canvasWidth = width.toDouble() * if (split) 2 else 1
        if (!point.x.isFinite() || !point.y.isFinite() || point.x < 0 || point.x >= canvasWidth ||
            point.y < 0 || point.y >= height.toDouble() ||
            (split && (if (point.x >= width.toDouble()) 1 else 0) != pane))
            throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
        return Point(point.x - pane * width.toDouble(), point.y)
    }
}

