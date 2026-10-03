package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import kotlin.math.ceil
import kotlin.math.floor

internal enum class ExportEncoding(val label: String, val mime: String, val extension: String) {
    Png8("PNG 8-bit", "image/png", "png"), Png16("PNG 16-bit", "image/png", "png"),
    Jpeg("JPEG", "image/jpeg", "jpg"), WebpLossless("WebP lossless", "image/webp", "webp"), WebpLossy("WebP lossy", "image/webp", "webp");
    fun format(quality: Int): ImageFormat = when (this) {
        Png8 -> ImageFormat.Png8; Png16 -> ImageFormat.Png16; Jpeg -> ImageFormat.Jpeg(quality.toUByte())
        WebpLossless -> ImageFormat.WebpLossless; WebpLossy -> ImageFormat.WebpLossy(quality.toUByte())
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

internal fun viewExportRegion(points: List<Point>, width: UInt, height: UInt): Rect {
    require(points.size == 4 && points.all { it.x.isFinite() && it.y.isFinite() })
    val left = floor(points.minOf { it.x }).coerceIn(0.0, width.toDouble())
    val top = floor(points.minOf { it.y }).coerceIn(0.0, height.toDouble())
    val right = ceil(points.maxOf { it.x }).coerceIn(0.0, width.toDouble())
    val bottom = ceil(points.maxOf { it.y }).coerceIn(0.0, height.toDouble())
    require(right > left && bottom > top) { "The current view does not intersect the image." }
    return Rect(left, top, right - left, bottom - top)
}
