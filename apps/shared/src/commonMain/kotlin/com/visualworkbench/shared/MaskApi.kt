package com.visualworkbench.shared

/** Native project capability. The project owns its worker; this facade owns no
 * separate native handle. Stop calls/collectors before closing the project. */
public expect fun WorkbenchProject.selections(): WorkbenchSelections

public interface WorkbenchSelections {
    public suspend fun document(documentId: String): SelectionDocument
    public suspend fun snapshot(binding: SelectionBinding, objectId: String,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): SelectionSnapshot
    /** Up to 64 regions, each at most 256x256. Gate the result by the exact
     * displayed project/document/revision before installing the overlay. */
    public suspend fun tiles(binding: SelectionBinding, selection: SelectionVersion,
        regions: List<SelectionRegion>, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): SelectionTiles
    /** One undoable operation. Capture binding at gesture start and use a new
     * transaction ID once. Conflict/ReusedTransaction require a refresh, never
     * automatic resubmission of old geometry against a new binding. Cancellation
     * settles native work; a transaction already admitted may have committed. */
    public suspend fun edit(options: SelectionEdit): SelectionReceipt
    /** Complete no-clobber PNG in caller-owned private staging. Cancellation
     * settles producer/file handles before it propagates; a completed file may
     * remain after the publication point and belongs to the caller's staging. */
    public suspend fun exportFile(options: SelectionExportOptions): SelectionExportReceipt
}

public data class SelectionBinding(
    public val projectId: String, public val documentId: String, public val sourceAssetId: String,
    public val hostSeq: ULong, public val stateHash: String,
)
public data class SelectionVersion(public val objectId: String, public val assetId: String, public val version: ULong)
public data class SelectionRegion(public val x: UInt, public val y: UInt, public val width: UInt, public val height: UInt)
public data class SelectionDocument(
    public val binding: SelectionBinding, public val width: UInt, public val height: UInt,
    public val sourceBitDepth: UInt, public val revision: ProjectInfo,
)
public data class SelectionSnapshot(
    public val binding: SelectionBinding, public val selection: SelectionVersion, public val layerId: String,
    public val width: UInt, public val height: UInt, public val nonzeroBounds: SelectionRegion?,
    public val editable: Boolean, public val visible: Boolean,
)
public data class SelectionTile(public val region: SelectionRegion, public val coverage: ByteArray)
public data class SelectionTiles(public val binding: SelectionBinding, public val selection: SelectionVersion,
    public val tiles: List<SelectionTile>)
public enum class SelectionCombine { Add, Subtract, Intersect }
public sealed interface SelectionTarget {
    public data class New(public val objectId: String, public val layerId: String) : SelectionTarget
    public data class Existing(public val selection: SelectionVersion) : SelectionTarget
}
public sealed interface SelectionOperation {
    public data class Rectangle(public val rectangle: Rect, public val combine: SelectionCombine) : SelectionOperation
    public data class Lasso(public val points: List<Point>, public val combine: SelectionCombine) : SelectionOperation
    /** Subtract is a mask eraser. Only real D-space samples; no motion prediction. */
    public data class Paint(public val points: List<Point>, public val radius: Double,
        public val opacity: UByte, public val combine: SelectionCombine) : SelectionOperation
    public data class Combine(public val other: SelectionVersion, public val combine: SelectionCombine) : SelectionOperation
    public data object Invert : SelectionOperation
    public data class Expand(public val radius: UInt) : SelectionOperation
    public data class Shrink(public val radius: UInt) : SelectionOperation
    public data class Feather(public val radius: UInt) : SelectionOperation
}
public data class SelectionEdit(
    public val binding: SelectionBinding, public val transactionId: String, public val deviceId: String,
    public val lamport: ULong, public val createdAtMs: Long, public val target: SelectionTarget,
    public val operation: SelectionOperation, public val memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL,
)
public data class SelectionReceipt(public val transactionId: String, public val snapshot: SelectionSnapshot,
    public val revision: ProjectInfo)
public enum class SelectionExportKind { Mask, Cutout }
public data class SelectionExportOptions(
    public val binding: SelectionBinding, public val selection: SelectionVersion,
    public val kind: SelectionExportKind, public val region: SelectionRegion?,
    public val workDirectory: String, public val outputPath: String,
    public val memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL,
    public val maxEncodedBytes: ULong = 64uL * 1024uL * 1024uL,
)
public data class SelectionExportReceipt(
    public val binding: SelectionBinding, public val selection: SelectionVersion,
    public val kind: SelectionExportKind, public val region: SelectionRegion,
    public val outputBitDepth: UByte, public val encodedBytes: ULong, public val blake3: String,
    public val metadataJson: String, public val revision: ProjectInfo, public val estimatedPeakBytes: ULong,
)
public enum class SelectionFailureKind {
    Invalid, Conflict, ReusedTransaction, Locked, Unsupported, Corrupt, Memory, Limit,
    EncodedLimit, Storage, Cancelled, Closed, Backpressure,
}
public class SelectionFailure(public val kind: SelectionFailureKind,
    public val estimatedBytes: ULong? = null, public val budgetBytes: ULong? = null,
    cause: Throwable? = null) : Exception(kind.name, cause)

/** A convenient overlay/export fence; controllers also retain their own project
 * attachment/event epoch so a closed and reopened project cannot accept a late UI result. */
public fun SelectionBinding.matches(info: ProjectInfo, displayedDocumentId: String): Boolean =
    projectId == info.projectId && documentId == displayedDocumentId &&
        hostSeq == info.hostSeq && stateHash == info.stateHash

internal fun SelectionEdit.ownedSelectionRequest(): SelectionEdit {
    val owned = when (val value = operation) {
        is SelectionOperation.Lasso -> {
            if (value.points.size !in 3..16_384) throw SelectionFailure(SelectionFailureKind.Limit)
            value.copy(points = value.points.toList())
        }
        is SelectionOperation.Paint -> {
            if (value.points.size !in 1..16_384) throw SelectionFailure(SelectionFailureKind.Limit)
            value.copy(points = value.points.toList())
        }
        else -> value
    }
    return copy(operation = owned)
}
