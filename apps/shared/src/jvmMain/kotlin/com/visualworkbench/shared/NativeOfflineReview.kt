package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.OfflineReviewBinding as NBinding
import com.visualworkbench.bindings.core.OfflineReviewChoice as NChoice
import com.visualworkbench.bindings.core.OfflineReviewProgress as NProgress
import com.visualworkbench.bindings.core.OfflineReviewValue as NValue
import com.visualworkbench.bindings.core.OfflineReviewDetails as NDetails
import com.visualworkbench.bindings.core.OfflineReviewMutation as NMutation

public actual fun offlineReviews(project: WorkbenchProject): WorkbenchOfflineReview =
    NativeOfflineReview(try { nativeProjectHandle(project) } catch (_: SessionFailure) { throw WorkflowFailure(WorkflowFailureKind.Invalid) })
private fun OfflineReviewBinding.native(): NBinding {
    if (projectId.length != 36 || stateHash.length != 64) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return NBinding(projectId, hostSeq, stateHash)
}
private fun NBinding.common(): OfflineReviewBinding = OfflineReviewBinding(projectId, hostSeq, stateHash)
private fun NValue.common(): OfflineReviewValue = OfflineReviewValue(preview, utf8Bytes, blake3, truncated, available)
private fun NProgress.common(): OfflineReviewProgress = when (this) {
    NProgress.UNREVIEWED -> OfflineReviewProgress.Unreviewed
    NProgress.KEPT -> OfflineReviewProgress.Kept
    NProgress.PREVIOUSLY_RESOLVED -> OfflineReviewProgress.PreviouslyResolved
    NProgress.DRAFT -> OfflineReviewProgress.Draft
    NProgress.QUEUED -> OfflineReviewProgress.Queued
    NProgress.APPLIED -> OfflineReviewProgress.Applied
}
private fun NDetails.common(): OfflineReviewDetails = OfflineReviewDetails(binding.common(), conflictId, objectId,
    property, deleteVsEdit, kept.common(), other.common(), current.common(), operation?.common(), transactionId,
    reviewed?.common(), progress.common(), nextLamport)
private fun NMutation.common(): OfflineReviewMutation = OfflineReviewMutation(details.common(), workflowInfo(project))
private class NativeOfflineReview(private val handle: ProjectSession) : WorkbenchOfflineReview {
    override suspend fun page(afterRow: Long, limit: UInt, memoryBudgetBytes: ULong): OfflineReviewPage = settledWorkflow { token ->
        handle.offlineReviewPage(afterRow, limit, memoryBudgetBytes, token).let {
            OfflineReviewPage(it.binding.common(), it.entries.map { entry -> OfflineReviewIndex(entry.row, entry.conflictId, entry.property, entry.deleteVsEdit) })
        }
    }
    override suspend fun details(conflictId: String, memoryBudgetBytes: ULong): OfflineReviewDetails = settledWorkflow { token ->
        handle.offlineReviewDetails(conflictId, memoryBudgetBytes, token).common()
    }
    override suspend fun stage(conflictId: String, expected: OfflineReviewBinding, metadata: WorkflowMetadata,
        choice: OfflineReviewChoice, memoryBudgetBytes: ULong): OfflineReviewMutation = settledWorkflow { token ->
        val command = when (choice) {
            OfflineReviewChoice.SwapOther -> NChoice.SwapOther
            is OfflineReviewChoice.RestoreCopy -> NChoice.RestoreCopy(choice.documentId, choice.objectId)
        }
        handle.stageOfflineReview(conflictId, expected.native(), workflowMetadata(metadata), command, memoryBudgetBytes, token).common()
    }
    override suspend fun submit(conflictId: String, expected: OfflineReviewBinding, memoryBudgetBytes: ULong): OfflineReviewSubmission = settledWorkflow { token ->
        handle.submitOfflineReview(conflictId, expected.native(), memoryBudgetBytes, token).let {
            OfflineReviewSubmission(WorkflowReceipt(it.receipt.transactionId, workflowInfo(it.receipt.revision), it.receipt.duplicate),
                it.progress.common(), it.metadataPending)
        }
    }
    override suspend fun keep(conflictId: String, expected: OfflineReviewBinding, memoryBudgetBytes: ULong): OfflineReviewMutation = settledWorkflow { token ->
        handle.keepOfflineReview(conflictId, expected.native(), memoryBudgetBytes, token).common()
    }
    override suspend fun cancelDraft(conflictId: String, expected: OfflineReviewBinding, memoryBudgetBytes: ULong): OfflineReviewMutation = settledWorkflow { token ->
        handle.cancelOfflineReviewDraft(conflictId, expected.native(), memoryBudgetBytes, token).common()
    }
}
