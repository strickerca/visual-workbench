package com.visualworkbench.android.editor

import java.util.Locale

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
