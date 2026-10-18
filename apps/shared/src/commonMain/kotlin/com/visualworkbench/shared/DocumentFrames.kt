package com.visualworkbench.shared

/** The complete authority captured before worker dispatch and bitmap creation. */
public data class DocumentFrameBinding(
    public val sourceAssetId: String,
    public val documentId: String,
    public val page: UInt,
    public val revision: ProjectInfo,
    public val attachmentEpoch: Long,
    public val viewportGeneration: Long,
)

/** Straight RGBA8 sRGB derivative; never an original or an export source.
 * The frame owner retains bytes unchanged until every drawing lease retires. */
public class DocumentDisplayTile(public val origin: Point, public val scale: Double,
    public val width: UInt, public val height: UInt, public val rgba: ByteArray) {
    public override fun toString(): String = "DocumentDisplayTile(width=$width, height=$height)"
}
public class DocumentDisplayFrame(public val binding: DocumentFrameBinding, public val tiles: List<DocumentDisplayTile>)

/** This is caller authority/shape admission, in addition to native nonce/hash/
 * metadata verification. Platform bitmap/texture memory requires its own reserve. */
public fun validateDocumentDisplayFrame(frame: DocumentDisplayFrame, current: DocumentFrameBinding): Unit {
    val before = frame.binding
    if (before.sourceAssetId != current.sourceAssetId || before.documentId != current.documentId ||
        before.page != current.page || before.attachmentEpoch != current.attachmentEpoch ||
        before.viewportGeneration != current.viewportGeneration || !before.revision.sameDocumentRevision(current.revision) ||
        before.documentId !in before.revision.documentIds || current.documentId !in current.revision.documentIds)
        throw DocumentFailure(DocumentFailureKind.Stale)
    if (before.sourceAssetId.length != 64 || before.sourceAssetId.any { it !in "0123456789abcdef" } ||
        before.attachmentEpoch < 0 || before.viewportGeneration < 0)
        throw DocumentFailure(DocumentFailureKind.Invalid)
    if (frame.tiles.size > 32) throw DocumentFailure(DocumentFailureKind.Limit)
    for (tile in frame.tiles) {
        if (tile.width !in 1u..512u || tile.height !in 1u..512u ||
            !tile.scale.isFinite() || tile.scale <= 0.0 ||
            !tile.origin.x.isFinite() || !tile.origin.y.isFinite() ||
            tile.rgba.size != tile.width.toInt() * tile.height.toInt() * 4)
            throw DocumentFailure(DocumentFailureKind.Verification)
    }
}
