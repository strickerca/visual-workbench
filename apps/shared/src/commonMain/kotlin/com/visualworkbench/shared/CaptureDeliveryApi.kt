package com.visualworkbench.shared

/** One immutable, exact source/target plan. The saved recovery capture remains
 * separate; commit appends its document to the current host project. Preparing
 * only owns private scratch. This does not establish transport/display timing. */
public data class CaptureDeliveryInfo(public val transactionId: String, public val source: WorkflowBinding,
    public val target: WorkflowBinding, public val documentId: String, public val sourceAssetId: String,
    public val captureSessionId: String, public val frameId: ULong, public val geometryRevision: UInt,
    public val encodedBytes: ULong, public val operationCount: UInt)
public interface WorkbenchCaptureDelivery {
    public fun describe(): CaptureDeliveryInfo
    /** Exact retries on this handle return the already durable receipt. */
    public suspend fun commit(): WorkflowReceipt
    public fun close()
}
public data class ReceivedCapture(public val binding: WorkflowBinding, public val createdHostSeq: ULong,
    public val sourceAssetId: String, public val captureSessionId: String, public val frameId: ULong,
    public val geometryRevision: UInt, public val originalVerified: Boolean)
public expect suspend fun prepareCaptureDelivery(source: WorkbenchProject, target: WorkbenchProject,
    sourceBinding: WorkflowBinding, targetBinding: WorkflowBinding, metadata: WorkflowMetadata,
    memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchCaptureDelivery
/** Coalesces only notifications. Every earlier capture remains a saved document.
 * Missing original returns originalVerified=false; no placeholder is success. */
public expect suspend fun WorkbenchProject.receivedCapture(afterHostSeq: ULong,
    memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): ReceivedCapture?
