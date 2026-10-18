package com.visualworkbench.android.editor

import com.visualworkbench.shared.ImageFormat
import org.junit.Assert.assertEquals
import org.junit.Test

class OrdinaryFormatSelectionTest {
    @Test fun bmpAndBothTiffDepthsUseActualFormatMimeAndSuffix() {
        assertEquals("image/bmp", ExportEncoding.Bmp8.mime)
        assertEquals("bmp", ExportEncoding.Bmp8.extension)
        assertEquals(ImageFormat.Bmp8, ExportEncoding.Bmp8.format(90))
        assertEquals("image/tiff", ExportEncoding.Tiff8.mime)
        assertEquals("image/tiff", ExportEncoding.Tiff16.mime)
        assertEquals("tiff", ExportEncoding.Tiff16.extension)
        assertEquals(ImageFormat.Tiff8, ExportEncoding.Tiff8.format(90))
        assertEquals(ImageFormat.Tiff16, ExportEncoding.Tiff16.format(90))
    }
    @Test fun choosingTiff16DoesNotGrantDepthReductionOrColorConversion() {
        val chosen = ExportSelection(encoding = ExportEncoding.Tiff16)
        assertEquals(false, chosen.allowDepthReduction)
        assertEquals(false, chosen.convertToSrgb)
        assertEquals(null, chosen.matteRgb)
    }
    @Test fun chooserMetadataAndAliasesFollowTheSelectedCodec() {
        for (encoding in listOf(ExportEncoding.Tiff8, ExportEncoding.Tiff16)) {
            assertEquals("image/tiff", encoding.exportMimeType())
            assertEquals("tiff", encoding.exportFileSuffix())
            for (suffix in listOf("tif", "tiff", ".TIF", "TIFF")) assertEquals(true, encoding.acceptsFileSuffix(suffix))
            for (suffix in listOf("png", "bmp", "", "tiff ", "image.tiff")) assertEquals(false, encoding.acceptsFileSuffix(suffix))
        }
        assertEquals("image/bmp", ExportEncoding.Bmp8.exportMimeType())
        assertEquals("bmp", ExportEncoding.Bmp8.exportFileSuffix())
        assertEquals(false, ExportEncoding.Bmp8.acceptsFileSuffix("png"))
        assertEquals(true, ExportEncoding.Jpeg.acceptsFileSuffix("jpeg"))
        assertEquals(true, ExportEncoding.Png16.acceptsFileSuffix("png"))
    }
}
