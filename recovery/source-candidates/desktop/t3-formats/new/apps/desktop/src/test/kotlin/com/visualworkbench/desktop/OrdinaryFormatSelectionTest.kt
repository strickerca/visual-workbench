package com.visualworkbench.desktop

import com.visualworkbench.shared.ImageFormat
import org.junit.Test
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse

class OrdinaryFormatSelectionTest {
    @Test fun formatsRetainTheirDepthAndCannotBecomePngClipboardExports() {
        for (encoding in listOf(ExportEncoding.Bmp8, ExportEncoding.Tiff8, ExportEncoding.Tiff16)) assertFalse(encoding.png)
        assertEquals(ImageFormat.Bmp8, ExportSettings(encoding = ExportEncoding.Bmp8).format())
        assertEquals(ImageFormat.Tiff8, ExportSettings(encoding = ExportEncoding.Tiff8).format())
        assertEquals(ImageFormat.Tiff16, ExportSettings(encoding = ExportEncoding.Tiff16).format())
    }
    @Test fun highDepthTiffDoesNotSilentlyChangeTheExportPolicies() {
        val chosen = ExportSettings(encoding = ExportEncoding.Tiff16)
        assertFalse(chosen.allowDepthReduction)
        assertFalse(chosen.convertToSrgb)
        assertFalse(chosen.assumeUntaggedSrgb)
        assertEquals(ExportMatte.Preserve, chosen.matte)
    }
}
