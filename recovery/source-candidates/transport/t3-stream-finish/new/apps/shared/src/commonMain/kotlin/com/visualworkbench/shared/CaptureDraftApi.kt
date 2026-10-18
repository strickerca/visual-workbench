package com.visualworkbench.shared

/** Requires the exact live link and its frozen receiver. Creates only private
 * temporary canonical storage; it does not replace the live project's link. */
public expect suspend fun ProjectLink.createCaptureDraft(stream: WorkbenchCaptureStream,
    documentId: String, layerId: String, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchCaptureDraft

public data class CaptureDraftInfo(public val draftId: String, public val documentId: String,
    public val frame: CaptureStreamFrame, public val previewAssetId: String, public val connectionEpoch: ULong)
public data class CaptureDraftSaveInfo(public val draft: CaptureDraftInfo, public val source: WorkflowBinding,
    public val target: WorkflowBinding, public val originalAssetId: String?, public val savedAssetId: String, public val degraded: Boolean, public val transactionId: String,
    public val operationCount: UInt)
public interface WorkbenchCaptureDraft {
    public val info: CaptureDraftInfo
    /** Ordinary local stroke/shape/undo facade, with no independent pairing. */
    public val project: WorkbenchProject
    /** Requires the lossless owner for this frame. Seals edits before the exact
     * snapshot. A discarded uncommitted plan restores draft editing. */
    public suspend fun prepareSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        original: CaptureFrozenBackground, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchCaptureDraftSave
    /** Explicitly saves the exact timed-out decoded preview with permanent
     * degraded metadata. Requires a visible confirmation; no original claim. */
    public suspend fun prepareDegradedSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        preview: CaptureFrozenBackground, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchCaptureDraftSave
    /** Joins native Save work and SQLite destruction before temporary cleanup.
     * Only after success may the caller invoke resumeAfterDraftDisposed(). */
    public suspend fun close(): Unit
}
public interface WorkbenchCaptureDraftSave {
    public val info: CaptureDraftSaveInfo
    /** Durable local transaction receipt; a client outbox may still be awaiting
     * host acknowledgement. An exact retry cannot append a second capture. */
    public suspend fun commit(): WorkflowReceipt
    public suspend fun close(): Unit
}