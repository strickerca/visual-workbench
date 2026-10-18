package com.visualworkbench.desktop

import java.util.Locale

/** File output metadata never makes a BMP/TIFF eligible for PNG-only transfer. */
internal fun ExportEncoding.exportMimeType(): String = when (this) {
    ExportEncoding.Png8, ExportEncoding.Png16 -> "image/png"
    ExportEncoding.Jpeg -> "image/jpeg"
    ExportEncoding.WebpLossless, ExportEncoding.WebpLossy -> "image/webp"
    ExportEncoding.Bmp8 -> "image/bmp"
    ExportEncoding.Tiff8, ExportEncoding.Tiff16 -> "image/tiff"
}
internal fun ExportEncoding.exportFileSuffix(): String = extension
internal fun ExportEncoding.acceptsFileSuffix(value: String): Boolean {
    val suffix = value.removePrefix(".").lowercase(Locale.ROOT)
    return when (this) {
        ExportEncoding.Jpeg -> suffix == "jpg" || suffix == "jpeg"
        ExportEncoding.Tiff8, ExportEncoding.Tiff16 -> suffix == "tif" || suffix == "tiff"
        else -> suffix == exportFileSuffix()
    }
}
