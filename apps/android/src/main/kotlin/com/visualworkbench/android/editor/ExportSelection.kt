package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import kotlin.math.ceil
import kotlin.math.floor

internal enum class ExportEncoding(val label: String, val mime: String, val extension: String) {
    Png8("PNG 8-bit", "image/png", "png"), Png16("PNG 16-bit", "image/png", "png"),
    Jpeg("JPEG", "image/jpeg", "jpg"), WebpLossless("WebP lossless", "image/webp", "webp"), WebpLossy("WebP lossy", "image/webp", "webp"),
    Bmp8("BMP 8-bit", "image/bmp", "bmp"), Tiff8("TIFF 8-bit", "image/tiff", "tiff"), Tiff16("TIFF 16-bit", "image/tiff", "tiff");
    val exportAvailable: Boolean get() = true
    fun format(quality: Int): ImageFormat = when (this) {
        Png8 -> ImageFormat.Png8; Png16 -> ImageFormat.Png16; Jpeg -> ImageFormat.Jpeg(quality.toUByte())
        WebpLossless -> ImageFormat.WebpLossless; WebpLossy -> ImageFormat.WebpLossy(quality.toUByte())
        Bmp8 -> ImageFormat.Bmp8; Tiff8 -> ImageFormat.Tiff8; Tiff16 -> ImageFormat.Tiff16
    }
}
internal data class ExportSelection(val marked: Boolean = true, val viewOnly: Boolean = false,
    val encoding: ExportEncoding = ExportEncoding.Png8, val quality: Int = 90, val whiteMatte: Boolean = false,
    val convertToSrgb: Boolean = false, val allowDepthReduction: Boolean = false) {
    val matteRgb: UInt? get() = if (encoding == ExportEncoding.Jpeg && whiteMatte) 0xffffffu else null
}

internal data class ExportReady(val projectId: String, val documentId: String, val stateHash: String,
    val hostSeq: ULong, val width: UInt, val height: UInt, val blake3: String) {
    fun matches(info: ProjectInfo?, doc: DocumentSnapshot?): Boolean = info?.projectId == projectId &&
        info.stateHash == stateHash && info.hostSeq == hostSeq && doc?.documentId == documentId
}

internal data class ExportTicket(val id: String, val projectId: String, val documentId: String,
    val hostSeq: ULong, val stateHash: String, val options: ExportOptions, val encoding: ExportEncoding) {
    fun matches(info: ProjectInfo?, doc: DocumentSnapshot?, checked: ExportOptions?): Boolean =
        info?.projectId == projectId && info.hostSeq == hostSeq && info.stateHash == stateHash &&
            doc?.documentId == documentId && checked == options
}

/** Interpret the retained source options in D, then use the revision-bound native O projection. */
internal suspend fun projectExportOptions(project: WorkbenchProject, binding: WorkflowBinding,
    sourceOptions: ExportOptions,
): Pair<ExportOptions, ImageOutputExportGeometry> =
    projectExportOptions(project, binding, sourceOptions, ::imageOutputExportGeometry)

internal suspend fun projectExportOptions(project: WorkbenchProject, binding: WorkflowBinding,
    sourceOptions: ExportOptions,
    geometryQuery: suspend (WorkbenchProject, WorkflowBinding, SelectionRegion?, ULong) -> ImageOutputExportGeometry,
): Pair<ExportOptions, ImageOutputExportGeometry> {
    require(sourceOptions.documentId == binding.documentId) { "Export document changed." }
    val source = sourceOptions.region?.let { rect ->
        fun coordinate(value: Double): UInt {
            require(value.isFinite() && value >= 0.0 && value <= UInt.MAX_VALUE.toDouble() && value % 1.0 == 0.0) {
                "Export region requires nonnegative integer source coordinates."
            }
            return value.toUInt()
        }
        SelectionRegion(coordinate(rect.x), coordinate(rect.y), coordinate(rect.width), coordinate(rect.height))
            .also { require(it.width > 0u && it.height > 0u) { "Export region is empty." } }
    }
    val geometry = geometryQuery(project, binding, source, sourceOptions.memoryBudgetBytes)
    require(geometry.binding == binding && geometry.sourceWidth > 0u && geometry.sourceHeight > 0u &&
        geometry.outputWidth > 0u && geometry.outputHeight > 0u &&
        geometry.encodedWidth > 0u && geometry.encodedHeight > 0u &&
        (source == null) == (geometry.region == null)) { "Export frame changed or returned invalid dimensions." }
    if (source != null) require(source.x.toULong() + source.width.toULong() <= geometry.sourceWidth.toULong() &&
        source.y.toULong() + source.height.toULong() <= geometry.sourceHeight.toULong()) {
        "Export region exceeds its source document."
    }
    val output = geometry.region
    if (output == null) {
        require(geometry.encodedWidth == geometry.outputWidth && geometry.encodedHeight == geometry.outputHeight) {
            "Full export frame returned inconsistent dimensions."
        }
    } else require(output.width > 0u && output.height > 0u &&
        output.x.toULong() + output.width.toULong() <= geometry.outputWidth.toULong() &&
        output.y.toULong() + output.height.toULong() <= geometry.outputHeight.toULong() &&
        geometry.encodedWidth == output.width && geometry.encodedHeight == output.height) {
        "Export frame returned an invalid output rectangle."
    }
    return sourceOptions.copy(region = output?.let {
        Rect(it.x.toDouble(), it.y.toDouble(), it.width.toDouble(), it.height.toDouble())
    }) to geometry
}

internal fun viewExportRegion(points: List<Point>, width: UInt, height: UInt): Rect {
    require(points.size == 4 && points.all { it.x.isFinite() && it.y.isFinite() })
    val left = floor(points.minOf { it.x }).coerceIn(0.0, width.toDouble())
    val top = floor(points.minOf { it.y }).coerceIn(0.0, height.toDouble())
    val right = ceil(points.maxOf { it.x }).coerceIn(0.0, width.toDouble())
    val bottom = ceil(points.maxOf { it.y }).coerceIn(0.0, height.toDouble())
    require(right > left && bottom > top) { "The current view does not intersect the image." }
    return Rect(left, top, right - left, bottom - top)
}
