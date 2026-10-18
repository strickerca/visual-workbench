package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.jetbrains.skia.ColorAlphaType
import org.jetbrains.skia.ColorSpace
import org.jetbrains.skia.ColorType
import org.jetbrains.skia.ImageInfo
import org.junit.Assert.*
import org.junit.Test

/** Skia CPU raster/tag storage only. No monitor, GPU, HDR or calibration proof. */
class ColorDisplayTest {
    private val binding = WorkflowBinding("00000000-0000-7000-8000-000000000001", "00000000-0000-7000-8000-000000000002", 7uL, "a".repeat(64))
    private fun request(space: DisplayColorSpace = DisplayColorSpace.DisplayP3) = DisplayColorRequest(binding, DisplayRegion(0u, 0u, 2u, 1u), space)
    private fun pixels(space: DisplayColorSpace = DisplayColorSpace.DisplayP3) = DisplayColorPixels(binding, "b".repeat(64), DisplayRegion(0u, 0u, 2u, 1u), space,
        byteArrayOf(255.toByte(), 0, 0, 255.toByte(), 40, 100, 200.toByte(), 128.toByte()), ByteArray(132), 8u, false, false, 52uL * 1024uL * 1024uL)
    @Test fun rasterStoresStraightAlphaAndTargetSpace() = runBlocking {
        val owned = ColorDisplay.bitmap(pixels(), request())
        try {
            assertEquals(ColorType.RGBA_8888, owned.skia.colorType); assertEquals(ColorAlphaType.UNPREMUL, owned.skia.alphaType)
            val target = org.jetbrains.skia.Bitmap()
            try {
                assertTrue(target.allocPixels(ImageInfo(2, 1, ColorType.RGBA_8888, ColorAlphaType.UNPREMUL, ColorSpace.displayP3)))
                assertTrue(owned.skia.readPixels(target)); assertArrayEquals(pixels().rgba, target.readPixels(ImageInfo(2, 1, ColorType.RGBA_8888, ColorAlphaType.UNPREMUL, ColorSpace.displayP3), 8, 0, 0))
            } finally { target.close() }
        } finally { owned.abandon() }
    }
    @Test fun srgbAndLegacyConvertedDerivativesCarrySrgbTag() = runBlocking {
        val p = pixels(DisplayColorSpace.Srgb)
        val tagged = ColorDisplay.bitmap(p, request(DisplayColorSpace.Srgb))
        val legacy = ColorDisplay.legacy(BackgroundImage(2u, 1u, p.rgba, p.sourceAssetId, 8u, byteArrayOf()))
        try { assertTrue(tagged.skia.colorSpace?.isSRGB == true); assertTrue(legacy.skia.colorSpace?.isSRGB == true) }
        finally { tagged.abandon(); legacy.abandon() }
    }
    @Test fun lateHashOrTargetMismatchIsRefusedBeforeSkiaOwnership() = runBlocking {
        for (p in listOf(pixels().copy(binding = binding.copy(stateHash = "c".repeat(64))), pixels().copy(space = DisplayColorSpace.Srgb))) {
            try { ColorDisplay.bitmap(p, request()).abandon(); fail("Expected refusal") } catch (error: DisplayColorFailure) { assertEquals(DisplayColorFailureKind.Invalid, error.kind) }
        }
    }
    @Test fun cancelledCallerCannotReceiveOrAdoptAnImage() = runBlocking {
        var delivered = false
        val job = launch { currentCoroutineContext().cancel(); try { ColorDisplay.bitmap(pixels(), request()).abandon(); delivered = true } catch (_: CancellationException) {} }
        job.join(); assertFalse(delivered)
    }
}
