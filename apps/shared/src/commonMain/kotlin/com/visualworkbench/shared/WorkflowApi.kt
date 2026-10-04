package com.visualworkbench.shared

/** A full visible-state fence: host sequence alone does not cover offline edits. */
public data class WorkflowBinding(public val projectId: String, public val documentId: String, public val hostSeq: ULong, public val stateHash: String)
public data class WorkflowMetadata(public val transactionId: String, public val deviceId: String, public val firstLamport: ULong, public val createdAtMs: Long)
public enum class WorkflowFailureKind { Invalid, Limit, Memory, Stale, Missing, Locked, Reconciliation, Detached, ReusedTransaction, Closed, Cancelled, Backpressure, Storage }
public class WorkflowFailure(public val kind: WorkflowFailureKind, public val estimatedBytes: ULong? = null, public val budgetBytes: ULong? = null, cause: Throwable? = null) : Exception(kind.name, cause)
public data class WorkflowPlanInfo(public val transactionId: String, public val expected: WorkflowBinding, public val operationCount: UInt, public val nextLamport: ULong)
public data class WorkflowReceipt(public val transactionId: String, public val revision: ProjectInfo, public val duplicate: Boolean)
/** Own and close this immutable plan. Preparing has no canonical side effects.
 * commit rechecks the visible fence in the same serialized worker as persistence.
 * Repeating commit on the same handle is exact-byte idempotent; after process
 * death inspect the durable transaction/revision, never blindly regenerate IDs.
 * Cancellation settles native work before returning; a commit already published
 * may be observed through project.changes or a retry of the still-owned plan. */
public interface WorkbenchWorkflowPlan {
    public fun describe(): WorkflowPlanInfo
    public suspend fun commit(): WorkflowReceipt
    public fun close()
}
