package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.McpInboxPixels
import com.visualworkbench.shared.McpInboxRegion
import com.visualworkbench.shared.McpInboxSide
import com.visualworkbench.shared.Rect
import org.junit.Assert.*
import org.junit.Test

class McpImageRegistrationTest {
    private fun texture(x: Long, y: Long): Int = (x.toInt() and 255) or
        ((y.toInt() and 255) shl 8) or (((x * 73 + y * 151).toInt() and 255) shl 16) or (255 shl 24)

    private fun tile(
        width: Int = 40, height: Int = 40, x: UInt = 0u, y: UInt = 0u,
        side: McpInboxSide = McpInboxSide.Before,
        canvasWidth: UInt = 512u, canvasHeight: UInt = 512u,
        color: (Long, Long) -> Int = ::texture,
    ): McpInboxPixels {
        val bytes = ByteArray(width * height * 4)
        for (row in 0 until height) for (column in 0 until width) {
            val value = color(x.toLong() + column, y.toLong() + row)
            val offset = (row * width + column) * 4
            for (channel in 0..3) bytes[offset + channel] = (value ushr (channel * 8)).toByte()
        }
        return McpInboxPixels("receipt", "a".repeat(64), side,
            McpInboxRegion(x, y, width.toUInt(), height.toUInt()), canvasWidth, canvasHeight, bytes)
    }

    private fun after(before: McpInboxPixels) = before.copy(side = McpInboxSide.After, rgbaSrgb = before.rgbaSrgb.copyOf())
    private fun alteredPixel(value: McpInboxPixels, x: Int, y: Int) {
        val offset = (y * value.region.width.toInt() + x) * 4
        value.rgbaSrgb[offset] = (value.rgbaSrgb[offset].toInt() xor 90).toByte()
    }
    private fun aligned(result: McpImageRegistrationResult): McpImageRegistrationResult.Aligned {
        assertTrue(result.toString(), result is McpImageRegistrationResult.Aligned)
        return result as McpImageRegistrationResult.Aligned
    }
    private fun refused(expected: McpImageRegistrationRefusal, result: McpImageRegistrationResult) {
        assertEquals(McpImageRegistrationResult.Refused(expected), result)
    }

    @Test fun exactTranslationUsesPixelsAndReportsTheVerifiedSupport() {
        val before = tile()
        val after = tile(side = McpInboxSide.After) { x, y -> texture(x - 3, y + 2) }
        val result = aligned(registerMcpImages(before, after, emptyList(), emptyList(), 4))
        assertEquals(3, result.dx); assertEquals(-2, result.dy)
        assertEquals(32 * 32, result.supportingPixels)
        assertEquals(32 * 32, result.sampledPixels)
        assertEquals(4, result.maxTranslationPixels)
        assertEquals("exact_protected_rgba_integer_translation", result.method)
    }

    @Test fun translationIncludesDistinctTileOriginsAndLargeUnsignedCoordinates() {
        val before = tile(x = 4_000_000_000u, y = 4_000_000_000u,
            canvasWidth = UInt.MAX_VALUE, canvasHeight = UInt.MAX_VALUE)
        val after = tile(x = 4_000_000_003u, y = 3_999_999_998u, side = McpInboxSide.After,
            canvasWidth = UInt.MAX_VALUE, canvasHeight = UInt.MAX_VALUE) { x, y -> texture(x - 4, y + 3) }
        val result = aligned(registerMcpImages(before, after, emptyList(), emptyList(), 4))
        assertEquals(4, result.dx); assertEquals(-3, result.dy)
    }

    @Test fun identityRequiresActualProtectedPixelEvidence() {
        val before = tile()
        val result = aligned(registerMcpImages(before, after(before), emptyList(), emptyList(), 2))
        assertEquals(0, result.dx); assertEquals(0, result.dy)
        val unrelated = tile(side = McpInboxSide.After) { x, y -> texture(x + 80, y + 80) }
        refused(McpImageRegistrationRefusal.NoMatch, registerMcpImages(before, unrelated, emptyList(), emptyList(), 2))
    }

    @Test fun declaredBeforeAndAfterChangesDoNotSupplyAlignmentEvidence() {
        val before = tile()
        val after = tile(side = McpInboxSide.After) { x, y -> texture(x - 3, y + 2) }
        for (y in 12..15) for (x in 19..22) alteredPixel(after, x, y)
        val result = aligned(registerMcpImages(before, after,
            listOf(Rect(16.0, 14.0, 4.0, 4.0)), listOf(Rect(19.0, 12.0, 4.0, 4.0)), 4))
        assertEquals(3, result.dx); assertEquals(-2, result.dy)
        assertTrue(result.supportingPixels < 32 * 32)
        refused(McpImageRegistrationRefusal.NoMatch, registerMcpImages(before, after, emptyList(), emptyList(), 4))
    }

    @Test fun partiallyCoveredPixelsAreExcludedAndAfterChangesAreExpandedForAllCandidates() {
        val before = tile()
        val after = after(before)
        alteredPixel(after, 20, 20)
        val result = aligned(registerMcpImages(before, after, emptyList(), listOf(Rect(20.25, 20.25, 0.5, 0.5)), 2))
        assertEquals(0, result.dx); assertEquals(0, result.dy)
        assertEquals(36 * 36 - 5 * 5, result.supportingPixels)
    }

    @Test fun finalVerificationFindsAProtectedChangeOutsideTheSampleSet() {
        val before = tile(width = 64, height = 64)
        val after = after(before)
        val protectedCount = 60 * 60
        val sampled = (0 until 2048).map { ((2L * it + 1) * protectedCount / (2L * 2048)).toInt() }.toSet()
        val missing = (0 until protectedCount).first { it !in sampled }
        alteredPixel(after, 2 + missing % 60, 2 + missing / 60)
        refused(McpImageRegistrationRefusal.NoMatch, registerMcpImages(before, after, emptyList(), emptyList(), 2))
    }

    @Test fun periodicTextureRefusesMultipleExactTranslations() {
        val before = tile { x, y -> ((x.toInt() and 3) or ((y.toInt() and 3) shl 2)) or (255 shl 24) }
        refused(McpImageRegistrationRefusal.Ambiguous,
            registerMcpImages(before, after(before), emptyList(), emptyList(), 4))
    }

    @Test fun uniformAndInvisiblePixelsCannotEstablishAlignment() {
        val uniform = tile { _, _ -> 0xff112233.toInt() }
        refused(McpImageRegistrationRefusal.InsufficientTexture,
            registerMcpImages(uniform, after(uniform), emptyList(), emptyList(), 2))
        val invisible = tile { x, y -> texture(x, y) and 0x00ffffff }
        refused(McpImageRegistrationRefusal.TooFewProtectedPixels,
            registerMcpImages(invisible, after(invisible), emptyList(), emptyList(), 2))
    }

    @Test fun insufficientCoverageAndSpatialSupportRefuse() {
        val before = tile()
        refused(McpImageRegistrationRefusal.ExcessiveChanges,
            registerMcpImages(before, after(before), listOf(Rect(0.0, 0.0, 36.0, 40.0)), emptyList(), 2))
        val tooFew = tile { x, y -> if (x in 10L..19L && y in 10L..19L) texture(x, y) else 0 }
        refused(McpImageRegistrationRefusal.TooFewProtectedPixels,
            registerMcpImages(tooFew, after(tooFew), emptyList(), emptyList(), 2))
        val line = tile(width = 24, height = 160) { x, y -> if (x == 10L && y in 10L..137L) texture(x, y) else 0 }
        refused(McpImageRegistrationRefusal.InsufficientSpatialSupport,
            registerMcpImages(line, after(line), emptyList(), emptyList(), 2))
    }

    @Test fun exactRgbaComparisonDoesNotTolerateAlphaChangesOrResizing() {
        val before = tile()
        val changedAlpha = tile(side = McpInboxSide.After) { x, y -> texture(x, y) and 0xfeffffff.toInt() }
        refused(McpImageRegistrationRefusal.NoMatch,
            registerMcpImages(before, changedAlpha, emptyList(), emptyList(), 2))
        val scaled = tile(side = McpInboxSide.After) { x, y -> texture(x / 2, y / 2) }
        refused(McpImageRegistrationRefusal.NoMatch,
            registerMcpImages(before, scaled, emptyList(), emptyList(), 2))
    }

    @Test fun missingCommonFootprintAndTranslationsOutsideTheSearchRefuse() {
        val before = tile()
        refused(McpImageRegistrationRefusal.OutOfBounds,
            registerMcpImages(before, tile(x = 100u, side = McpInboxSide.After), emptyList(), emptyList(), 2))
        val shiftedBeyondBound = tile(side = McpInboxSide.After) { x, y -> texture(x - 5, y) }
        refused(McpImageRegistrationRefusal.NoMatch,
            registerMcpImages(before, shiftedBeyondBound, emptyList(), emptyList(), 4))
    }

    @Test fun malformedFramesAndUnboundedOrInvalidChangesAreRejectedBeforeRegistration() {
        val before = tile()
        refused(McpImageRegistrationRefusal.InvalidFrame,
            registerMcpImages(before.copy(rgbaSrgb = ByteArray(0)), after(before), emptyList(), emptyList(), 2))
        refused(McpImageRegistrationRefusal.InvalidFrame,
            registerMcpImages(before.copy(region = McpInboxRegion(0u, 0u, 513u, 40u)), after(before), emptyList(), emptyList(), 2))
        refused(McpImageRegistrationRefusal.InvalidFrame,
            registerMcpImages(before.copy(region = McpInboxRegion(UInt.MAX_VALUE, 0u, 40u, 40u), canvasWidth = UInt.MAX_VALUE),
                after(before), emptyList(), emptyList(), 2))
        for (invalid in listOf(Rect(Double.NaN, 0.0, 1.0, 1.0), Rect(0.0, 0.0, 0.0, 1.0), Rect(511.5, 0.0, 1.0, 1.0))) {
            refused(McpImageRegistrationRefusal.InvalidChangeRegions,
                registerMcpImages(before, after(before), listOf(invalid), emptyList(), 2))
        }
        refused(McpImageRegistrationRefusal.InvalidChangeRegions,
            registerMcpImages(before, after(before), emptyList(), List(65) { Rect(0.0, 0.0, 1.0, 1.0) }, 2))
        for (radius in listOf(0, 33, Int.MAX_VALUE)) {
            refused(McpImageRegistrationRefusal.InvalidSearchBound,
                registerMcpImages(before, after(before), emptyList(), emptyList(), radius))
        }
    }

    @Test fun cancellationBeforeAdmissionAndDuringCandidateWorkReturnsNoAlignment() {
        val before = tile()
        refused(McpImageRegistrationRefusal.Cancelled,
            registerMcpImages(before, after(before), emptyList(), emptyList(), 4) { true })
        var checks = 0
        refused(McpImageRegistrationRefusal.Cancelled,
            registerMcpImages(before, after(before), emptyList(), emptyList(), 4) { ++checks > 120 })
        assertTrue(checks > 120)
    }
}
