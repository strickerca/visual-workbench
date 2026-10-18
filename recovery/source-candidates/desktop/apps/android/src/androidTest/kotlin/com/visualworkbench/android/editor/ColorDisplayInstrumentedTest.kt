package com.visualworkbench.android.editor

import android.graphics.ColorSpace
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer

/** Actual CPU bitmap/tag storage only. No screen gamut or GPU fidelity claim. */
@RunWith(AndroidJUnit4::class)
class ColorDisplayInstrumentedTest {
    private val binding = WorkflowBinding("00000000-0000-7000-8000-000000000001", "00000000-0000-7000-8000-000000000002", 7uL, "a".repeat(64))
    private fun request(space: DisplayColorSpace = DisplayColorSpace.DisplayP3) = DisplayColorRequest(binding, DisplayRegion(0u, 0u, 2u, 1u), space)
    private fun pixels(space: DisplayColorSpace = DisplayColorSpace.DisplayP3) = DisplayColorPixels(binding, "b".repeat(64), DisplayRegion(0u, 0u, 2u, 1u), space,
        byteArrayOf(255.toByte(), 0, 0, 255.toByte(), 40, 100, 200.toByte(), 128.toByte()), ByteArray(132), 8u, false, false, 52uL * 1024uL * 1024uL)
    @Test fun wideTagAndPremultipliedRawBytesArePreserved() = runBlocking {
        val bitmap = ColorDisplay.bitmap(pixels(), request())
        try {
            assertEquals(ColorSpace.get(ColorSpace.Named.DISPLAY_P3), bitmap.colorSpace); assertTrue(bitmap.isPremultiplied)
            val buffer = ByteBuffer.allocate(bitmap.byteCount); bitmap.copyPixelsToBuffer(buffer)
            assertArrayEquals(byteArrayOf(255.toByte(), 0, 0, 255.toByte(), 20, 50, 100, 128.toByte()), buffer.array())
        } finally { bitmap.recycle() }
    }
    @Test fun srgbResponseUsesSrgbTagRatherThanWideRetagging() = runBlocking {
        val bitmap = ColorDisplay.bitmap(pixels(DisplayColorSpace.Srgb), request(DisplayColorSpace.Srgb))
        try { assertEquals(ColorSpace.get(ColorSpace.Named.SRGB), bitmap.colorSpace) } finally { bitmap.recycle() }
    }
    @Test fun staleOrRetaggedResponsesCannotAllocateAPlatformImage() = runBlocking {
        for (value in listOf(pixels().copy(binding = binding.copy(stateHash = "c".repeat(64))), pixels().copy(space = DisplayColorSpace.Srgb))) {
            try { ColorDisplay.bitmap(value, request()).recycle(); fail("Expected refusal") }
            catch (error: DisplayColorFailure) { assertEquals(DisplayColorFailureKind.Invalid, error.kind) }
        }
    }
    @Test fun anAlreadyCancelledConversionReturnsNoBitmap() = runBlocking {
        var delivered = false
        val job = launch { currentCoroutineContext().cancel(); try { ColorDisplay.bitmap(pixels(), request()).recycle(); delivered = true } catch (_: CancellationException) {} }
        job.join(); assertFalse(delivered)
    }
    @Test fun legacyConvertedDerivativeRemainsExplicitlySrgb() = runBlocking {
        val value = pixels(); val bitmap = ColorDisplay.legacy(BackgroundImage(2u, 1u, value.rgba, value.sourceAssetId, 8u, byteArrayOf()))
        try { assertEquals(ColorSpace.get(ColorSpace.Named.SRGB), bitmap.colorSpace); assertEquals(2, bitmap.width) } finally { bitmap.recycle() }
    }
}
