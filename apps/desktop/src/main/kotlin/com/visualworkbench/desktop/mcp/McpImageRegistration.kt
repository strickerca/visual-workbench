package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.McpInboxPixels
import com.visualworkbench.shared.Rect
import kotlin.math.ceil
import kotlin.math.floor

internal enum class McpImageRegistrationRefusal {
    InvalidFrame, InvalidChangeRegions, InvalidSearchBound, OutOfBounds,
    TooFewProtectedPixels, ExcessiveChanges, InsufficientSpatialSupport,
    InsufficientTexture, Ambiguous, NoMatch, WorkLimit, Cancelled,
}

/** An image correspondence observation only. It supplies no capture authority,
 * owner verdict, marker identity or exterior-pixel proof. */
internal sealed class McpImageRegistrationResult {
    /** Maps BEFORE package pixel (x,y) to AFTER package pixel (x+dx,y+dy).
     * Tile origins are included. The caller must separately convert package
     * pixels to canonical source coordinates using admitted source scales.
     * Uniqueness is limited to the stated integer translation search. */
    data class Aligned(
        val dx: Int, val dy: Int, val supportingPixels: Int, val sampledPixels: Int,
        val maxTranslationPixels: Int,
        val method: String = "exact_protected_rgba_integer_translation",
    ) : McpImageRegistrationResult()

    data class Refused(val reason: McpImageRegistrationRefusal) : McpImageRegistrationResult()
}

/** Explicit bounds, independent of caller-controlled canvas dimensions.
 * At most 4,225 candidates, 2,048 sample comparisons per candidate and 262,144
 * final protected comparisons. Exclusion writes are bounded by 128*262,144.
 * The work budget counts pixel copies, visits, writes and comparisons; it is
 * not a wall-clock guarantee. Internal primitive-array payload stays below
 * 2.5 MiB (two RGBA snapshots, one exclusion mask and two sample arrays).
 * Cancellation is checked before allocation, between snapshot/mask rows, and
 * at most every 1,024 counted units in the remaining loops. */
internal object McpImageRegistrationLimits {
    const val MAX_TILE_EDGE = 512
    const val MAX_CHANGE_REGIONS_PER_FRAME = 64
    const val MAX_TRANSLATION_PIXELS = 32
    const val MAX_SAMPLES = 2048
    const val MIN_PROTECTED_PIXELS = 128
    const val MIN_DISTINCT_SAMPLE_COLORS = 16
    const val MIN_SUPPORT_EDGE = 8
    const val MAX_WORK_UNITS = 44_000_000L
}

/** Bounded, translation-only fallback for two real lossless RGBA capture tiles.
 * The caller owns capture identity, consent, color/depth conversion and scale;
 * inbox receipt labels and equal dimensions do not authenticate a recapture.
 * Do not mutate either input byte array while its initial snapshot is copied.
 *
 * Changes use each frame's package coordinates. A partially covered pixel is
 * excluded in full. The common footprint fits every searched translation, and
 * AFTER changes are expanded over the entire search, so candidates cannot win
 * by discarding inconvenient pixels or moving them into a declared change.
 * At least half that footprint must remain outside changes; transparent source
 * pixels do not supply anchors. Deterministic spread samples reject candidates;
 * a unique survivor must also match every protected pixel in the footprint.
 * Multiple sample survivors conservatively remain unresolved, even if a larger
 * search or more samples might distinguish them. No interpolation or tolerance
 * can silently turn display conversion, resize or changed pixels into a match.
 */
internal fun registerMcpImages(
    before: McpInboxPixels, after: McpInboxPixels,
    beforeChanges: List<Rect>, afterChanges: List<Rect>,
    maxTranslationPixels: Int = McpImageRegistrationLimits.MAX_TRANSLATION_PIXELS,
    isCancelled: () -> Boolean = { false },
): McpImageRegistrationResult {
    fun refused(reason: McpImageRegistrationRefusal) = McpImageRegistrationResult.Refused(reason)
    val work = RegistrationWork(isCancelled)
    try {
        work.checkCancellation()
        if (maxTranslationPixels !in 1..McpImageRegistrationLimits.MAX_TRANSLATION_PIXELS) {
            return refused(McpImageRegistrationRefusal.InvalidSearchBound)
        }
        val sourceInput = registrationTile(before) ?: return refused(McpImageRegistrationRefusal.InvalidFrame)
        val targetInput = registrationTile(after) ?: return refused(McpImageRegistrationRefusal.InvalidFrame)
        if (!validRegistrationChanges(beforeChanges, before) || !validRegistrationChanges(afterChanges, after)) {
            return refused(McpImageRegistrationRefusal.InvalidChangeRegions)
        }
        val radius = maxTranslationPixels.toLong()
        val left = maxOf(sourceInput.x, targetInput.x + radius)
        val top = maxOf(sourceInput.y, targetInput.y + radius)
        val right = minOf(sourceInput.x + sourceInput.width, targetInput.x + targetInput.width - radius)
        val bottom = minOf(sourceInput.y + sourceInput.height, targetInput.y + targetInput.height - radius)
        if (right <= left || bottom <= top || (right - left) * (bottom - top) < McpImageRegistrationLimits.MIN_PROTECTED_PIXELS) {
            return refused(McpImageRegistrationRefusal.OutOfBounds)
        }

        val source = sourceInput.snapshot(work)
        val target = targetInput.snapshot(work)
        val excluded = ByteArray(source.width * source.height)
        for (change in beforeChanges) excludeRegistrationRegion(excluded, source, change, 0, work)
        for (change in afterChanges) excludeRegistrationRegion(excluded, source, change, maxTranslationPixels, work)

        val firstX = (left - source.x).toInt()
        val firstY = (top - source.y).toInt()
        val endX = (right - source.x).toInt()
        val endY = (bottom - source.y).toInt()
        val footprintPixels = (endX - firstX) * (endY - firstY)
        var outsideChanges = 0
        var protectedPixels = 0
        var minX = endX; var minY = endY; var maxX = -1; var maxY = -1
        for (y in firstY until endY) for (x in firstX until endX) {
            work.advance()
            val index = y * source.width + x
            if (excluded[index].toInt() != 0) continue
            outsideChanges++
            if (source.rgba[index * 4 + 3].toInt() == 0) continue
            protectedPixels++
            minX = minOf(minX, x); minY = minOf(minY, y)
            maxX = maxOf(maxX, x); maxY = maxOf(maxY, y)
        }
        if (outsideChanges * 2 < footprintPixels) return refused(McpImageRegistrationRefusal.ExcessiveChanges)
        if (protectedPixels < McpImageRegistrationLimits.MIN_PROTECTED_PIXELS) {
            return refused(McpImageRegistrationRefusal.TooFewProtectedPixels)
        }
        if (maxX - minX + 1 < McpImageRegistrationLimits.MIN_SUPPORT_EDGE ||
            maxY - minY + 1 < McpImageRegistrationLimits.MIN_SUPPORT_EDGE) {
            return refused(McpImageRegistrationRefusal.InsufficientSpatialSupport)
        }

        val sampleCount = minOf(McpImageRegistrationLimits.MAX_SAMPLES, protectedPixels)
        val sampleColors = IntArray(sampleCount)
        val targetIndices = IntArray(sampleCount)
        val distinctColors = IntArray(McpImageRegistrationLimits.MIN_DISTINCT_SAMPLE_COLORS)
        var distinctCount = 0
        var ordinal = 0
        var sample = 0
        fun nextOrdinal(): Int = ((2L * sample + 1) * protectedPixels / (2L * sampleCount)).toInt()
        var wanted = nextOrdinal()
        for (y in firstY until endY) for (x in firstX until endX) {
            work.advance()
            val index = y * source.width + x
            if (excluded[index].toInt() != 0 || source.rgba[index * 4 + 3].toInt() == 0) continue
            if (ordinal++ != wanted || sample == sampleCount) continue
            val color = registrationPixel(source.rgba, index)
            sampleColors[sample] = color
            targetIndices[sample] = (source.y + y - target.y).toInt() * target.width + (source.x + x - target.x).toInt()
            if (distinctCount < distinctColors.size) {
                work.advance(distinctCount + 1)
                var seen = false
                for (i in 0 until distinctCount) if (distinctColors[i] == color) { seen = true; break }
                if (!seen) distinctColors[distinctCount++] = color
            }
            sample++
            if (sample < sampleCount) wanted = nextOrdinal()
        }
        if (distinctCount < distinctColors.size) return refused(McpImageRegistrationRefusal.InsufficientTexture)

        var found = false
        var foundX = 0; var foundY = 0
        for (dy in -maxTranslationPixels..maxTranslationPixels) for (dx in -maxTranslationPixels..maxTranslationPixels) {
            work.checkCancellation()
            val offset = dy * target.width + dx
            var matches = true
            for (i in 0 until sampleCount) {
                work.advance()
                if (sampleColors[i] != registrationPixel(target.rgba, targetIndices[i] + offset)) {
                    matches = false; break
                }
            }
            if (matches) {
                if (found) return refused(McpImageRegistrationRefusal.Ambiguous)
                found = true; foundX = dx; foundY = dy
            }
        }
        if (!found) return refused(McpImageRegistrationRefusal.NoMatch)

        for (y in firstY until endY) for (x in firstX until endX) {
            work.advance()
            val index = y * source.width + x
            if (excluded[index].toInt() != 0 || source.rgba[index * 4 + 3].toInt() == 0) continue
            val targetIndex = (source.y + y + foundY - target.y).toInt() * target.width +
                (source.x + x + foundX - target.x).toInt()
            if (registrationPixel(source.rgba, index) != registrationPixel(target.rgba, targetIndex)) {
                return refused(McpImageRegistrationRefusal.NoMatch)
            }
        }
        work.checkCancellation()
        return McpImageRegistrationResult.Aligned(foundX, foundY, protectedPixels, sampleCount, maxTranslationPixels)
    } catch (stopped: RegistrationStopped) {
        return refused(stopped.reason)
    }
}

private class RegistrationStopped(val reason: McpImageRegistrationRefusal) : RuntimeException()

private class RegistrationWork(private val isCancelled: () -> Boolean) {
    private var remaining = McpImageRegistrationLimits.MAX_WORK_UNITS
    private var sinceCancellation = 0
    fun checkCancellation() {
        if (isCancelled()) throw RegistrationStopped(McpImageRegistrationRefusal.Cancelled)
        sinceCancellation = 0
    }
    fun advance(units: Int = 1) {
        if (units.toLong() > remaining) throw RegistrationStopped(McpImageRegistrationRefusal.WorkLimit)
        remaining -= units
        if (sinceCancellation + units >= 1024) checkCancellation()
        sinceCancellation += units
    }
}

private data class RegistrationTile(val x: Long, val y: Long, val width: Int, val height: Int, val rgba: ByteArray) {
    fun snapshot(work: RegistrationWork): RegistrationTile {
        work.checkCancellation()
        val bytes = ByteArray(rgba.size)
        for (row in 0 until height) {
            work.checkCancellation(); work.advance(width)
            rgba.copyInto(bytes, row * width * 4, row * width * 4, (row + 1) * width * 4)
        }
        return copy(rgba = bytes)
    }
}

private fun registrationTile(pixels: McpInboxPixels): RegistrationTile? {
    val region = pixels.region
    if (region.width !in 1u..McpImageRegistrationLimits.MAX_TILE_EDGE.toUInt() ||
        region.height !in 1u..McpImageRegistrationLimits.MAX_TILE_EDGE.toUInt() ||
        pixels.canvasWidth == 0u || pixels.canvasHeight == 0u ||
        region.x.toLong() + region.width.toLong() > pixels.canvasWidth.toLong() ||
        region.y.toLong() + region.height.toLong() > pixels.canvasHeight.toLong()) return null
    val width = region.width.toInt(); val height = region.height.toInt()
    if (pixels.rgbaSrgb.size != width * height * 4) return null
    return RegistrationTile(region.x.toLong(), region.y.toLong(), width, height, pixels.rgbaSrgb)
}

private fun validRegistrationChanges(changes: List<Rect>, pixels: McpInboxPixels): Boolean {
    if (changes.size > McpImageRegistrationLimits.MAX_CHANGE_REGIONS_PER_FRAME) return false
    return changes.all { region ->
        region.x.isFinite() && region.y.isFinite() && region.width.isFinite() && region.height.isFinite() &&
            region.x >= 0 && region.y >= 0 && region.width > 0 && region.height > 0 &&
            (region.x + region.width).isFinite() && (region.y + region.height).isFinite() &&
            region.x + region.width > region.x && region.y + region.height > region.y &&
            region.x + region.width <= pixels.canvasWidth.toDouble() &&
            region.y + region.height <= pixels.canvasHeight.toDouble()
    }
}

private fun excludeRegistrationRegion(
    excluded: ByteArray, source: RegistrationTile, region: Rect, expansion: Int, work: RegistrationWork,
) {
    val left = (floor(region.x - expansion).coerceIn(source.x.toDouble(), (source.x + source.width).toDouble()) - source.x).toInt()
    val top = (floor(region.y - expansion).coerceIn(source.y.toDouble(), (source.y + source.height).toDouble()) - source.y).toInt()
    val right = (ceil(region.x + region.width + expansion).coerceIn(source.x.toDouble(), (source.x + source.width).toDouble()) - source.x).toInt()
    val bottom = (ceil(region.y + region.height + expansion).coerceIn(source.y.toDouble(), (source.y + source.height).toDouble()) - source.y).toInt()
    for (y in top until bottom) {
        work.checkCancellation(); work.advance(right - left)
        excluded.fill(1.toByte(), y * source.width + left, y * source.width + right)
    }
}

private fun registrationPixel(bytes: ByteArray, index: Int): Int {
    val offset = index * 4
    return (bytes[offset].toInt() and 255) or ((bytes[offset + 1].toInt() and 255) shl 8) or
        ((bytes[offset + 2].toInt() and 255) shl 16) or ((bytes[offset + 3].toInt() and 255) shl 24)
}
