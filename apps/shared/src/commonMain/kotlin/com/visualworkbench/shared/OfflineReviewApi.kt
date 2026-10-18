package com.visualworkbench.shared

/** Proposed additive seam, subject to the coordinator's ABI admission. Project
 * lifetime and transport authority stay with the existing WorkbenchProject. */
public data class OfflineReviewBinding(public val projectId: String, public val hostSeq: ULong, public val stateHash: String)
public fun ProjectInfo.offlineReviewBinding(): OfflineReviewBinding = OfflineReviewBinding(projectId, hostSeq, stateHash)
public data class OfflineReviewIndex(public val row: Long, public val conflictId: String,
    public val property: String = "", public val deleteVsEdit: Boolean = false)
public data class OfflineReviewPage(public val binding: OfflineReviewBinding, public val entries: List<OfflineReviewIndex>)
public enum class OfflineReviewProgress { Unreviewed, Kept, PreviouslyResolved, Draft, Queued, Applied }
public sealed interface OfflineReviewChoice {
    public data object SwapOther : OfflineReviewChoice
    public data class RestoreCopy(public val documentId: String, public val objectId: String) : OfflineReviewChoice
}
/** An explicit private preview, never a log/diagnostic field. Truncation is
 * visible and prevents new submission through this bounded review surface. */
public data class OfflineReviewValue(public val preview: String, public val utf8Bytes: ULong,
    public val blake3: String?, public val truncated: Boolean, public val available: Boolean) {
    override fun toString(): String = "OfflineReviewValue([PRIVATE])"
}
public data class OfflineReviewDetails(public val binding: OfflineReviewBinding, public val conflictId: String,
    public val objectId: String, public val property: String, public val deleteVsEdit: Boolean,
    public val kept: OfflineReviewValue, public val other: OfflineReviewValue, public val current: OfflineReviewValue,
    public val operation: OfflineReviewValue?, public val transactionId: String?, public val reviewed: OfflineReviewBinding?,
    public val progress: OfflineReviewProgress, public val nextLamport: ULong) {
    override fun toString(): String = "OfflineReviewDetails([PRIVATE])"
    public val staleDraft: Boolean get() = progress == OfflineReviewProgress.Draft && reviewed != binding
    public val completePreview: Boolean get() = listOfNotNull(kept, other, current, operation).let {
        operation != null && it.all { value -> value.available && !value.truncated }
    }
}
public data class OfflineReviewMutation(public val details: OfflineReviewDetails, public val project: ProjectInfo)
public data class OfflineReviewSubmission(public val receipt: WorkflowReceipt, public val progress: OfflineReviewProgress,
    public val metadataPending: Boolean)
public interface WorkbenchOfflineReview {
    public suspend fun page(afterRow: Long, limit: UInt = 32u, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): OfflineReviewPage
    public suspend fun details(conflictId: String, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): OfflineReviewDetails
    /** Saves a durable intent, without submitting or recording an undo event. */
    public suspend fun stage(conflictId: String, expected: OfflineReviewBinding, metadata: WorkflowMetadata,
        choice: OfflineReviewChoice, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): OfflineReviewMutation
    /** Exact saved identity only. Queued/accepted retries never regenerate IDs. */
    public suspend fun submit(conflictId: String, expected: OfflineReviewBinding,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): OfflineReviewSubmission
    public suspend fun keep(conflictId: String, expected: OfflineReviewBinding,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): OfflineReviewMutation
    public suspend fun cancelDraft(conflictId: String, expected: OfflineReviewBinding,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): OfflineReviewMutation
}
public expect fun offlineReviews(project: WorkbenchProject): WorkbenchOfflineReview

/** Human-readable action scope; unsupported journal internals stay out of labels. */
public fun offlineReviewPropertyLabel(property: String): String = when {
    property == "documents.definition.title" -> "Document title"
    property == "layers.definition.name" -> "Layer name"
    property == "layers.definition.order_key" -> "Layer order"
    property.startsWith("layers.") -> "Layer setting"
    property == "objects.state.style.width" -> "Annotation width"
    property.startsWith("objects.state.style.stroke") -> "Stroke color"
    property.startsWith("objects.state.style.fill") -> "Fill color"
    property.startsWith("objects.state.style") -> "Annotation style"
    property == "objects.state.hidden" -> "Annotation visibility"
    property == "objects.state.locked" -> "Annotation lock"
    property.startsWith("objects.state.transform") -> "Annotation position"
    property.startsWith("objects.state.shape") -> "Annotation content or geometry"
    property.startsWith("objects.") -> "Annotation change"
    else -> "Retained change"
}
