package com.visualworkbench.android.editor

import com.visualworkbench.shared.CreateFileProject
import com.visualworkbench.shared.ImageFormat
import com.visualworkbench.shared.WorkbenchProject
import com.visualworkbench.shared.WorkbenchStreamingCore
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.fail
import org.junit.Test

class OrdinaryFormatSelectionTest {
    @Test fun ordinarySuffixesRetainImageIntake() {
        for (suffix in listOf("png", "jpg", "jpeg", "webp", "heic", "heif", "avif", "bmp", "tif", "tiff", ".TIF", "TIFF")) {
            assertEquals(OrdinaryImportKind.Image, ordinaryImportKind(suffix))
        }
    }
    @Test fun compositesPreviewsAndAiRequestTheirExplicitImportRoute() {
        for (suffix in listOf("psd", "psb", ".PSD", "PSB")) assertEquals(OrdinaryImportKind.FlattenedComposite, ordinaryImportKind(suffix))
        for (suffix in listOf("dng", "cr2", ".DNG", "CR2")) assertEquals(OrdinaryImportKind.RawPreview, ordinaryImportKind(suffix))
        for (suffix in listOf("nef", "arw", "NEF", "ARW")) assertEquals(OrdinaryImportKind.Unsupported, ordinaryImportKind(suffix))
        for (suffix in listOf("ai", ".AI")) assertEquals(OrdinaryImportKind.PdfCompatibleAi, ordinaryImportKind(suffix))
    }
    @Test fun suffixSelectionDoesNotGuessFromNamesBytesOrUnadmittedRawVariants() {
        for (suffix in listOf("", ".", "..psd", "image.psd", "photo.dng", "dng ", " psb", "raw", "raf", "orf", "rw2", "raw-preview", "image/x-adobe-dng", "II*", "8BPS", "pdf", "svg", "mp4")) {
            assertEquals(OrdinaryImportKind.Unsupported, ordinaryImportKind(suffix))
        }
    }
    @Test fun providerNameAndExactMimeSelectOnlyAdmittedRoutes() {
        assertEquals(OrdinaryImportKind.FlattenedComposite,ordinaryImportKindForFile("portrait.PSD","application/octet-stream"))
        assertEquals(OrdinaryImportKind.RawPreview,ordinaryImportKindForFile("photo.CR2","application/octet-stream"))
        assertEquals(OrdinaryImportKind.Image,ordinaryImportKindForFile("Imported image","image/jpeg"))
        assertEquals(OrdinaryImportKind.RawPreview,ordinaryImportKindForFile("Imported image","image/x-adobe-dng"))
        assertEquals(OrdinaryImportKind.Unsupported,ordinaryImportKindForFile("photo.NEF","application/octet-stream"))
        assertEquals(OrdinaryImportKind.PdfCompatibleAi,ordinaryImportKindForFile("drawing.ai","application/pdf"))
        assertEquals(OrdinaryImportKind.Unsupported,ordinaryImportKindForFile("Imported image","application/octet-stream"))
    }
    @Test fun ordinaryAndCompositeRoutesPreserveTheOwnedStagingRequest() = runBlocking {
        val request = CreateFileProject("project", "project-id", "document-id", "layer-id", "device-id", "fixture", "source", "work", 0L)
        val reached = IllegalStateException("owned ordinary route")
        var calls = 0
        val core = object : WorkbenchStreamingCore {
            override suspend fun createFile(options: CreateFileProject): WorkbenchProject {
                calls++
                assertSame(request, options)
                throw reached
            }
        }
        for (kind in listOf(OrdinaryImportKind.Image, OrdinaryImportKind.FlattenedComposite)) {
            try { createOrdinaryImageProject(kind, core, request); fail("The native route should be reached") }
            catch (error: IllegalStateException) { assertSame(reached, error) }
        }
        assertEquals(2, calls)
    }
    @Test fun aiAndUnsupportedSuffixesCannotFallThroughToOrdinaryImageDecoding() = runBlocking {
        val options = CreateFileProject("project", "project-id", "document-id", "layer-id", "device-id", "fixture", "source", "work", 0L)
        val core = object : WorkbenchStreamingCore {
            override suspend fun createFile(options: CreateFileProject): WorkbenchProject = error("Image decoding must not run")
        }
        for (kind in listOf(OrdinaryImportKind.PdfCompatibleAi, OrdinaryImportKind.Unsupported)) {
            try { createOrdinaryImageProject(kind, core, options); fail("A dedicated or unsupported route must refuse") }
            catch (error: IllegalArgumentException) {
                assertEquals(if (kind == OrdinaryImportKind.PdfCompatibleAi) "Import PDF-compatible AI through the PDF document importer." else "This image import suffix is unsupported.", error.message)
            }
        }
    }
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
