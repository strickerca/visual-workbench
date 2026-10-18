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
    @Test fun chooserMetadataAndAliasesNeverSubstituteAnotherFormat() {
        for (encoding in listOf(ExportEncoding.Tiff8, ExportEncoding.Tiff16)) {
            assertEquals("image/tiff", encoding.exportMimeType())
            assertEquals("tiff", encoding.exportFileSuffix())
            for (suffix in listOf("tif", "tiff", ".TIF", "TIFF")) assertEquals(true, encoding.acceptsFileSuffix(suffix))
            for (suffix in listOf("png", "bmp", "", "tiff ", "image.tiff")) assertEquals(false, encoding.acceptsFileSuffix(suffix))
        }
        assertEquals("image/bmp", ExportEncoding.Bmp8.exportMimeType())
        assertEquals("bmp", ExportEncoding.Bmp8.exportFileSuffix())
        assertEquals(false, ExportEncoding.Bmp8.png)
        assertEquals(false, ExportEncoding.Bmp8.acceptsFileSuffix("png"))
        assertEquals(true, ExportEncoding.Jpeg.acceptsFileSuffix("jpeg"))
        assertEquals(true, ExportEncoding.Png16.png)
    }
}
