package com.visualworkbench.android.session

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.*
import org.junit.Test
import java.nio.ByteBuffer

class QrLuminanceTest {
    @Test fun oversizedNegotiatedS23FrameStillDecodesQrWithinAllocationLimit() {
        // Synthetic credential-free data, using the observed negotiated size.
        val text = "vw-pair:v1:" + "A".repeat(380)
        val size = 2736
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, size, size)
        val source = ByteArray(size * size) { index -> if (matrix[index % size, index / size]) 0 else (-1).toByte() }
        val sample = checkNotNull(sampleQrLuminance(ByteBuffer.wrap(source), size, 1, size, size, 0, 0, size, size))
        try {
            assertEquals(1368, sample.width)
            assertEquals(1368, sample.height)
            assertTrue(sample.bytes.size <= 2_000_000)
            val reader = MultiFormatReader().apply { setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE))) }
            assertEquals(text, reader.decodeWithState(BinaryBitmap(HybridBinarizer(PlanarYUVLuminanceSource(
                sample.bytes, sample.width, sample.height, 0, 0, sample.width, sample.height, false)))).text)
        } finally { source.fill(0); sample.bytes.fill(0) }
    }

    @Test fun cropUsesPlaneOriginRowPaddingAndPixelStrideWithoutChangingBuffer() {
        val plane = ByteBuffer.allocate(43)
        plane.position(7)
        // Four pixels per row, pixel stride two, and two bytes of row padding.
        for (y in 0..2) for (x in 0..3) plane.put(7 + y * 10 + x * 2, (y * 4 + x).toByte())
        val sample = checkNotNull(sampleQrLuminance(plane, 10, 2, 4, 3, 1, 1, 3, 2))
        assertArrayEquals(byteArrayOf(5, 6, 7, 9, 10, 11), sample.bytes)
        assertEquals(7, plane.position())
        assertEquals(43, plane.limit())
        sample.bytes.fill(0)
    }

    @Test fun malformedCropOrTruncatedPlaneIsRefusedBeforeAllocation() {
        val plane = ByteBuffer.allocate(16)
        assertNull(sampleQrLuminance(plane, 4, 1, 4, 4, 1, 0, 4, 4))
        assertNull(sampleQrLuminance(plane, 4, 1, 4, 4, 0, 0, Int.MAX_VALUE, 4))
        assertNull(sampleQrLuminance(plane, 3, 1, 4, 4, 0, 0, 4, 4))
        assertNull(sampleQrLuminance(plane, 4, 1, 4, 4, 0, 0, 0, 4))
        plane.position(1)
        assertNull(sampleQrLuminance(plane, 4, 1, 4, 4, 0, 0, 4, 4))
    }
}
