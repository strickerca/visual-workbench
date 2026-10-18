package com.visualworkbench.android.remote

import org.junit.Assert.*
import org.junit.Test

class DecoderOutputGeometryTest {
    @Test fun paint_hevc_picture_padding_preserves_even_coded_and_odd_visible_geometry() {
        val scope = DecoderScope(1uL, "session", 1uL, "target", 1u)
        val config = DecoderConfig(scope, 1uL, 2076, 1512, 2076, 1511,
            byteArrayOf(1, 2), byteArrayOf(3, 4), byteArrayOf(5, 6))
        assertTrue(validDecoderConfig(config))
        val output = decoderOutputDimensions(2080, 1512, 0, 2075, 0, 1511)
        assertEquals(Pair(config.codedWidth, config.codedHeight), output)
        assertNotEquals(Pair(config.visibleWidth, config.visibleHeight), output)
    }

    @Test fun absent_crop_retains_raw_dimensions_including_mismatched_picture() {
        assertEquals(Pair(2076, 1512), decoderOutputDimensions(2076, 1512, null, null, null, null))
        // Without crop evidence, the existing exact coded-size guard must refuse this.
        assertEquals(Pair(2080, 1512), decoderOutputDimensions(2080, 1512, null, null, null, null))
    }

    @Test fun complete_crop_uses_inclusive_edges_on_both_axes() {
        assertEquals(Pair(100, 100), decoderOutputDimensions(100, 100, 0, 99, 0, 99))
        assertEquals(Pair(2076, 1512), decoderOutputDimensions(2080, 1520, 0, 2075, 0, 1511))
        assertEquals(Pair(2074, 1512), decoderOutputDimensions(2080, 1512, 0, 2073, 0, 1511))
    }

    @Test fun every_partial_crop_is_refused_instead_of_defaulted() {
        for (mask in 1..14) {
            assertNull("partial crop mask=$mask", decoderOutputDimensions(2080, 1512,
                if (mask and 1 != 0) 0 else null,
                if (mask and 2 != 0) 2075 else null,
                if (mask and 4 != 0) 0 else null,
                if (mask and 8 != 0) 1511 else null))
        }
    }

    @Test fun translated_origins_are_refused_even_with_matching_effective_size() {
        assertNull(decoderOutputDimensions(2080, 1512, 4, 2079, 0, 1511))
        assertNull(decoderOutputDimensions(2080, 1520, 0, 2075, 8, 1519))
    }

    @Test fun negative_reversed_out_of_bounds_and_extreme_crops_are_refused() {
        for (crop in listOf(
            listOf(-1, 2074, 0, 1511), listOf(0, 2075, -1, 1510),
            listOf(0, -1, 0, 1511), listOf(0, 2075, 0, -1),
            listOf(0, 2080, 0, 1511), listOf(0, 2075, 0, 1512),
            listOf(0, Int.MAX_VALUE, 0, 1511), listOf(0, 2075, 0, Int.MAX_VALUE),
            listOf(Int.MIN_VALUE, 2075, 0, 1511), listOf(0, 2075, Int.MIN_VALUE, 1511))) {
            assertNull("crop=$crop", decoderOutputDimensions(2080, 1512, crop[0], crop[1], crop[2], crop[3]))
        }
    }

    @Test fun invalid_or_unbounded_raw_dimensions_are_refused_before_crop_arithmetic() {
        for (side in listOf(Int.MIN_VALUE, -1, 0, 1, 4097, Int.MAX_VALUE)) {
            assertNull(decoderOutputDimensions(side, 1512, null, null, null, null))
            assertNull(decoderOutputDimensions(2080, side, null, null, null, null))
            assertNull(decoderOutputDimensions(side, 1512, 0, 2075, 0, 1511))
        }
    }
}
