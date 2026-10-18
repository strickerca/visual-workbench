@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

/** Immutable originals and display tiles are distinct. A job never receives a
 * destination URI/path, project database, ambient resource resolver or network. */
public enum class EditableKind { Pdf, Svg }
public data class DocumentPage(public val index: UInt, public val width: Double, public val height: Double,
    public val crop: List<Double>, public val quarterTurns: UInt, public val layerId: String)
public data class EditableDescriptor(public val documentId: String, public val sourceAssetId: String,
    public val kind: EditableKind, public val pages: List<DocumentPage>, public val revision: ProjectInfo)
public data class EditableSource(public val descriptor: EditableDescriptor, public val bytes: ByteArray)
public data class CreateDocument(public val path: String, public val projectId: String, public val documentId: String,
    public val deviceId: String, public val title: String, public val source: ByteArray, public val sourceBlake3: String,
    public val kind: EditableKind, public val pages: List<DocumentPage>, public val nowMs: Long, public val password: String? = null)
public interface WorkbenchDocumentsCore { public suspend fun createDocument(options: CreateDocument): WorkbenchProject }
public interface WorkbenchDocumentsProject {
    public suspend fun documentDescriptor(documentId: String): EditableDescriptor?
    public suspend fun documentSource(documentId: String, expected: ProjectInfo): EditableSource
    public suspend fun documentPage(documentId: String, page: UInt): DocumentSnapshot
    public suspend fun documentOverlays(documentId: String, page: UInt, expected: ProjectInfo): List<DocumentOverlay>
    public suspend fun publishDocumentCopy(documentId: String, destination: String, copy: DocumentExport): String
    public fun documentPassword(documentId: String): String?
    public fun documentPassword(documentId: String, password: String?)
    public fun selectedDocumentPage(documentId: String): UInt
    /** Local view state only. The editor fences gestures and revision adoption. */
    public suspend fun selectDocumentPage(documentId: String, page: UInt): DocumentSnapshot
}
public data class DocumentTileRequest(public val origin: Point, public val scale: Double, public val width: UInt, public val height: UInt)
public data class DocumentOverlay(public val page: UInt, public val contours: List<List<Point>>, public val transform: Transform, public val rgba: UInt)
public data class DocumentRegion(public val page: UInt, public val rectangle: Rect)
public enum class DocumentCopy { Original, Annotated, Flattened, PcRedaction, EveryPageRasterRedaction, SvgAnnotations }
public sealed interface DocumentOperation {
    public data object PdfInfo : DocumentOperation
    public data object SvgInfo : DocumentOperation
    public data class Page(public val index: UInt) : DocumentOperation
    public data class Tile(public val kind: EditableKind, public val page: UInt, public val tile: DocumentTileRequest) : DocumentOperation
    public data class Text(public val page: UInt, public val start: UInt, public val count: UInt = 4096u) : DocumentOperation
    public data class Selection(public val page: UInt, public val rectangle: Rect) : DocumentOperation
    public data class Copy(public val kind: DocumentCopy, public val overlays: List<DocumentOverlay> = emptyList(),
        public val regions: List<DocumentRegion> = emptyList(), public val scale: Double = 2.0,
        public val explicitlyRemoveEncryption: Boolean = false) : DocumentOperation
}
public data class DocumentRequest(public val operation: DocumentOperation, public val password: String? = null)
public data class DocumentGlyph(public val index: UInt, public val text: String?, public val bounds: Rect?, public val generated: Boolean)
public sealed interface DocumentMetadata {
    public data class Info(public val kind: EditableKind, public val pages: UInt, public val width: Double?, public val height: Double?, public val encrypted: Boolean, public val notices: List<String>) : DocumentMetadata
    public data class Page(public val index: UInt, public val width: Double, public val height: Double, public val crop: List<Double>, public val quarterTurns: UInt) : DocumentMetadata
    public data class Tile(public val page: UInt?, public val origin: Point, public val scale: Double, public val width: UInt, public val height: UInt) : DocumentMetadata
    public data class Text(public val page: UInt, public val glyphs: List<DocumentGlyph>, public val next: UInt?) : DocumentMetadata
    public data class Copy(public val mode: DocumentCopy, public val bytes: ULong, public val redacted: Boolean, public val notices: List<String>, public val planBlake3: String?) : DocumentMetadata
}
public data class DocumentReply(public val sourceBlake3: String, public val outputBlake3: String, public val metadata: DocumentMetadata, public val bytes: ByteArray)
public enum class DocumentFailureKind { Unavailable, Invalid, Limit, Busy, Cancelled, Timeout, Isolation, Verification, Password, Unsupported, Malformed, Worker, Stale }
public class DocumentFailure(public val kind: DocumentFailureKind, cause: Throwable? = null) : Exception(kind.name, cause)
public interface DocumentJobs {
    public val pcRedaction: Boolean
    /** Must await worker retirement even when cancellation races completion. */
    public suspend fun run(request: DocumentRequest, source: ByteArray): DocumentReply
    public suspend fun close()
}
public expect fun installDocumentJobs(jobs: DocumentJobs)
public expect fun installedDocumentJobs(): DocumentJobs
public data class DocumentExport(public val bytes: ByteArray, public val blake3: String, public val mode: DocumentCopy,
    public val redacted: Boolean, public val sourceAssetId: String, public val revision: ProjectInfo, public val notices: List<String>)
public fun ProjectInfo.sameDocumentRevision(other: ProjectInfo): Boolean = projectId == other.projectId && hostSeq == other.hostSeq && stateHash == other.stateHash && deviceId == other.deviceId && nextLamport == other.nextLamport
