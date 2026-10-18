package com.visualworkbench.android.editor

import com.visualworkbench.shared.CreateFileProject
import com.visualworkbench.shared.WorkbenchProject
import com.visualworkbench.shared.WorkbenchStreamingCore
import com.visualworkbench.shared.createRawPreviewProject
import java.util.Locale

internal enum class OrdinaryImportKind { Image, FlattenedComposite, RawPreview, PdfCompatibleAi, Unsupported }

/** Explicit suffix selection is UI intent; native byte admission remains decoder authority.
 * RAW selection requests only a bounded embedded preview, never RAW development or TIFF fallback.
 * PDF, SVG and video keep their existing dedicated import routes; AI uses the existing PDF route.
 */
internal fun ordinaryImportKind(suffix: String): OrdinaryImportKind = when (suffix.removePrefix(".").lowercase(Locale.ROOT)) {
    "png", "jpg", "jpeg", "webp", "heic", "heif", "avif", "bmp", "tif", "tiff" -> OrdinaryImportKind.Image
    "psd", "psb" -> OrdinaryImportKind.FlattenedComposite
    "dng", "cr2", "nef", "arw" -> OrdinaryImportKind.RawPreview
    "ai" -> OrdinaryImportKind.PdfCompatibleAi
    else -> OrdinaryImportKind.Unsupported
}

internal suspend fun createOrdinaryImageProject(kind: OrdinaryImportKind, core: WorkbenchStreamingCore, options: CreateFileProject): WorkbenchProject = when (kind) {
    OrdinaryImportKind.Image, OrdinaryImportKind.FlattenedComposite -> core.createFile(options)
    OrdinaryImportKind.RawPreview -> createRawPreviewProject(options)
    OrdinaryImportKind.PdfCompatibleAi -> throw IllegalArgumentException("Import PDF-compatible AI through the PDF document importer.")
    OrdinaryImportKind.Unsupported -> throw IllegalArgumentException("This image import suffix is unsupported.")
}

/** File chooser metadata follows the selected codec; it grants no conversions. */
internal fun ExportEncoding.exportMimeType(): String = mime
internal fun ExportEncoding.exportFileSuffix(): String = extension
internal fun ExportEncoding.acceptsFileSuffix(value: String): Boolean {
    val suffix = value.removePrefix(".").lowercase(Locale.ROOT)
    return when (this) {
        ExportEncoding.Jpeg -> suffix == "jpg" || suffix == "jpeg"
        ExportEncoding.Tiff8, ExportEncoding.Tiff16 -> suffix == "tif" || suffix == "tiff"
        else -> suffix == exportFileSuffix()
    }
}
