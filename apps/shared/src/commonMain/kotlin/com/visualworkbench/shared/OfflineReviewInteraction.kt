package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

public data class OfflineReviewAttachment(public val project: WorkbenchProject, public val info: ProjectInfo, public val epoch: Long)
public data class OfflineReviewState(public val entries: List<OfflineReviewIndex> = emptyList(),
    public val selected: OfflineReviewDetails? = null, public val afterRow: Long = 0,
    public val hasMore: Boolean = false, public val busy: Boolean = false,
    public val requiresRefresh: Boolean = false, public val message: String? = null)

/** UI-dispatcher owner borrowing the project and its existing edit gate. Reads
 * never submit saved drafts. Native work is settled before this operation slot
 * retires; detach joins the actual task before the project may close. */
public class OfflineReviewInteraction(
    private val scope: CoroutineScope,
    private val gate: Mutex,
    private val attachment: () -> OfflineReviewAttachment?,
    private val admitted: () -> Boolean,
    private val metadata: suspend (WorkbenchProject) -> WorkflowMetadata,
    private val newObjectId: () -> String,
    private val updated: suspend (OfflineReviewAttachment, ProjectInfo) -> Unit,
    private val committed: suspend (OfflineReviewAttachment, WorkflowReceipt) -> Unit,
    private val reviews: (WorkbenchProject) -> WorkbenchOfflineReview = ::offlineReviews,
) {
    private val mutable: MutableStateFlow<OfflineReviewState> = MutableStateFlow(OfflineReviewState())
    public val state: StateFlow<OfflineReviewState> = mutable
    private var work: Job? = null
    private var lifetime: Long = 0
    private var detaching: Boolean = false
    private var reload: Boolean = false

    private fun attached(a: OfflineReviewAttachment): Boolean = attachment()?.let {
        it.project === a.project && it.epoch == a.epoch
    } == true
    private fun current(a: OfflineReviewAttachment): Boolean = attached(a) &&
        attachment()?.info?.offlineReviewBinding() == a.info.offlineReviewBinding()
    private fun checkCurrent(a: OfflineReviewAttachment): Unit {
        if (!current(a)) throw WorkflowFailure(WorkflowFailureKind.Stale)
    }
    private fun selected(a: OfflineReviewAttachment): OfflineReviewDetails {
        checkCurrent(a)
        val value = state.value.selected ?: throw WorkflowFailure(WorkflowFailureKind.Missing)
        if (value.binding != a.info.offlineReviewBinding() || state.value.requiresRefresh)
            throw WorkflowFailure(WorkflowFailureKind.Stale)
        return value
    }
    private fun launch(mutation: Boolean = false, block: suspend (OfflineReviewAttachment, WorkbenchOfflineReview) -> Unit): Unit {
        if (detaching || !scope.isActive) return
        if (work?.isCompleted == false) return
        val a = attachment() ?: run { mutable.value = OfflineReviewState(); return }
        if (mutation && !admitted()) {
            mutable.value = state.value.copy(message = "Finish the current edit or dialog before reviewing a conflict.")
            return
        }
        val owned = lifetime
        mutable.value = state.value.copy(busy = true, message = null)
        val next = scope.launch(start = CoroutineStart.LAZY) {
            try {
                gate.withLock {
                    if (owned != lifetime) return@withLock
                    checkCurrent(a)
                    if (mutation && !admitted()) throw WorkflowFailure(WorkflowFailureKind.Backpressure)
                    val api = reviews(a.project)
                    val conflictId = state.value.selected?.conflictId
                    try { block(a, api) }
                    catch (failure: Exception) {
                        // settledWorkflow may discard a completed DTO when
                        // cancellation wins delivery after native persistence.
                        // Reconcile the exact saved identity while still owning
                        // the edit gate; never invent a new transaction.
                        if (mutation && conflictId != null) withContext(NonCancellable) {
                            reconcile(a, api, conflictId)
                        }
                        throw failure
                    }
                }
            } catch (cancel: CancellationException) {
                if (owned == lifetime && attached(a)) mutable.value = state.value.copy(
                    message = "Operation stopped. Saved reviews and committed edits are retained; refresh to inspect their status.")
                throw cancel
            } catch (failure: Exception) {
                if (owned == lifetime && attached(a)) mutable.value = state.value.copy(
                    requiresRefresh = state.value.requiresRefresh || (failure as? WorkflowFailure)?.kind == WorkflowFailureKind.Stale,
                    message = offlineReviewFailureMessage(failure))
            }
        }
        work = next
        next.invokeOnCompletion {
            if (work === next) {
                work = null
                if (!detaching && owned == lifetime) {
                    mutable.value = state.value.copy(busy = false)
                    if (reload) { reload = false; refresh() }
                }
            }
        }
        next.start()
    }
    private suspend fun reconcile(a: OfflineReviewAttachment, api: WorkbenchOfflineReview, conflictId: String): Unit {
        if (detaching || !attached(a)) return
        try {
            val detail = api.details(conflictId)
            val info = a.project.info()
            if (detaching || !attached(a)) return
            updated(a, info)
            if (!attached(a)) return
            if (detail.binding != info.offlineReviewBinding()) {
                mutable.value = state.value.copy(requiresRefresh = true)
                return
            }
            if (detail.progress in setOf(OfflineReviewProgress.Queued, OfflineReviewProgress.Applied)) {
                val transaction = detail.transactionId ?: throw WorkflowFailure(WorkflowFailureKind.Storage)
                committed(a, WorkflowReceipt(transaction, info, duplicate = true))
            }
            if (attached(a)) mutable.value = state.value.copy(selected = detail, requiresRefresh = false)
        } catch (_: Exception) {
            if (attached(a) && !detaching) mutable.value = state.value.copy(requiresRefresh = true)
        }
    }
    /** Root calls after a project/OPS publication. Busy work keeps ownership;
     * one deferred read refreshes its result after actual retirement. */
    public fun refresh(): Unit {
        if (detaching) return
        if (work?.isCompleted == false) { reload = true; return }
        val cursor = state.value.afterRow
        launch { a, api -> load(a, api, cursor) }
    }
    public fun firstPage(): Unit {
        if (state.value.busy) return
        launch { a, api -> load(a, api, 0) }
    }
    public fun nextPage(): Unit {
        if (!state.value.hasMore || state.value.busy) return
        val cursor = state.value.entries.lastOrNull()?.row ?: return
        launch { a, api -> load(a, api, cursor) }
    }
    private suspend fun load(a: OfflineReviewAttachment, api: WorkbenchOfflineReview, cursor: Long): Unit {
        val page = api.page(cursor, 32u)
        checkCurrent(a)
        if (page.binding != a.info.offlineReviewBinding()) throw WorkflowFailure(WorkflowFailureKind.Stale)
        val old = state.value.selected
        val detail = old?.let { api.details(it.conflictId) }
        checkCurrent(a)
        if (detail != null && detail.binding != page.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
        mutable.value = state.value.copy(entries = page.entries.toList(), selected = detail,
            afterRow = cursor, hasMore = page.entries.size == 32, requiresRefresh = false)
    }
    public fun select(conflictId: String): Unit = launch { a, api ->
        val detail = api.details(conflictId)
        checkCurrent(a)
        if (detail.binding != a.info.offlineReviewBinding()) throw WorkflowFailure(WorkflowFailureKind.Stale)
        mutable.value = state.value.copy(selected = detail, requiresRefresh = false)
    }
    public fun stageSwap(): Unit = stage { _, _ -> OfflineReviewChoice.SwapOther }
    /** The user explicitly chooses the displayed destination before staging.
     * Normal model validation requires the retained layer to belong to it. */
    public fun stageRestore(documentId: String): Unit = stage { a, value ->
        if (!value.deleteVsEdit || documentId !in a.info.documentIds) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        OfflineReviewChoice.RestoreCopy(documentId, newObjectId())
    }
    private fun stage(choice: (OfflineReviewAttachment, OfflineReviewDetails) -> OfflineReviewChoice): Unit = launch(true) { a, api ->
        val value = selected(a)
        if (value.progress != OfflineReviewProgress.Unreviewed) throw WorkflowFailure(WorkflowFailureKind.ReusedTransaction)
        val command = choice(a, value)
        val meta = metadata(a.project)
        checkCurrent(a)
        val result = api.stage(value.conflictId, value.binding, meta, command)
        withContext(NonCancellable) {
            if (attached(a)) {
                updated(a, result.project)
                if (attached(a)) mutable.value = state.value.copy(selected = result.details, requiresRefresh = false,
                    message = "Review saved. Inspect the prepared operation, then submit explicitly.")
            }
        }
    }
    public fun submit(): Unit = launch(true) { a, api ->
        val value = selected(a)
        if (value.progress !in setOf(OfflineReviewProgress.Draft, OfflineReviewProgress.Queued, OfflineReviewProgress.Applied))
            throw WorkflowFailure(WorkflowFailureKind.Invalid)
        if (value.staleDraft) throw WorkflowFailure(WorkflowFailureKind.Stale)
        if (value.progress == OfflineReviewProgress.Draft && !value.completePreview) throw WorkflowFailure(WorkflowFailureKind.Limit)
        val result = api.submit(value.conflictId, value.binding)
        // Native receipt bookkeeping settles even if cancellation arrives after
        // persistence. The callback receives the exact old attachment owner.
        withContext(NonCancellable) {
            committed(a, result.receipt)
            if (attached(a)) {
                mutable.value = state.value.copy(selected = value.copy(progress = result.progress,
                    binding = result.receipt.revision.offlineReviewBinding()), requiresRefresh = false,
                    message = if (result.metadataPending) "Edit saved. Refresh to settle its review status."
                        else if (result.progress == OfflineReviewProgress.Queued) "Queued for the existing paired sync link. The draft and original base are preserved."
                        else "Review edit saved. Use ordinary Undo to reverse it.")
                reload = true
            }
        }
    }
    public fun keep(): Unit = change(keep = true)
    public fun cancelDraft(): Unit = change(keep = false)
    private fun change(keep: Boolean): Unit = launch(true) { a, api ->
        val value = selected(a)
        if (keep && value.progress != OfflineReviewProgress.Unreviewed) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        if (!keep && value.progress != OfflineReviewProgress.Draft) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        val result = if (keep) api.keep(value.conflictId, value.binding) else api.cancelDraft(value.conflictId, value.binding)
        withContext(NonCancellable) {
            if (attached(a)) {
                updated(a, result.project)
                if (attached(a)) mutable.value = state.value.copy(selected = result.details, requiresRefresh = false,
                    message = if (keep) "Current value kept. The other value remains in the conflict record."
                        else "Saved review cancelled. The original conflict remains available.")
            }
        }
    }
    public fun cancelOperation(): Unit { work?.cancel() }
    /** Closing this surface does not delete durable review drafts. */
    public suspend fun detach(): Unit {
        detaching = true; lifetime++; reload = false
        withContext(NonCancellable) {
            val prior = work; prior?.cancelAndJoin()
            if (work === prior) work = null
            mutable.value = OfflineReviewState(busy = true)
        }
    }
    public fun resume(): Unit {
        check(work?.isCompleted != false)
        detaching = false
        mutable.value = OfflineReviewState()
    }
}
public fun offlineReviewFailureMessage(error: Exception): String = when ((error as? WorkflowFailure)?.kind) {
    WorkflowFailureKind.Stale -> "The project changed or has pending edits. The saved review is retained; refresh before choosing again."
    WorkflowFailureKind.Locked -> "The destination is locked. The conflict and saved review are preserved."
    WorkflowFailureKind.Missing, WorkflowFailureKind.Detached -> "The destination is unavailable. The retained conflict was not deleted."
    WorkflowFailureKind.ReusedTransaction -> "A saved review or transaction already exists. Refresh and inspect its exact status."
    WorkflowFailureKind.Memory, WorkflowFailureKind.Limit -> "This review exceeds the bounded view or memory limit. Its values are preserved."
    WorkflowFailureKind.Invalid -> "This review choice or destination is unavailable. The retained conflict is preserved."
    WorkflowFailureKind.Closed -> "The project closed. Reopen it to inspect saved reviews."
    WorkflowFailureKind.Backpressure -> "Finish current work, then try the review again."
    else -> "Review work could not finish. Saved drafts and any committed edit are retained; refresh to inspect their status."
}
