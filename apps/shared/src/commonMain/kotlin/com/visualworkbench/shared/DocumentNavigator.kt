package com.visualworkbench.shared

import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** A request borrows the editor's existing project and revision. It opens no
 * project and owns no rendering or document lifecycle. The root must recheck
 * [DocumentNavigator.matches] under its mutation lock before adopting a target. */
public class DocumentNavigationRequest internal constructor(
    public val binding: WorkflowBinding,
    public val attachmentEpoch: Long,
    public val documentId: String,
)

public data class DocumentNavigationState(
    public val active: Boolean = false,
    public val binding: WorkflowBinding? = null,
    public val attachmentEpoch: Long? = null,
    public val documentIds: List<String> = emptyList(),
    public val title: String = "",
    public val busy: Boolean = false,
    public val issue: String? = null,
) {
    public val ready: Boolean get() = binding != null && !busy
}

/** UI-dispatcher-only chooser over the complete catalog already supplied by
 * ProjectInfo. No second document manager, background reader, history or cache.
 * A pending request remains caller-owned until finish, cancellation or rebind. */
public class DocumentNavigator {
    private val mutable: MutableStateFlow<DocumentNavigationState> = MutableStateFlow(DocumentNavigationState())
    public val state: StateFlow<DocumentNavigationState> = mutable
    private var pending: DocumentNavigationRequest? = null

    public fun bind(document: DocumentSnapshot, epoch: Long) {
        val revision = document.render.revision
        val binding = WorkflowBinding(revision.projectId, document.documentId, revision.hostSeq, revision.stateHash)
        val ids = revision.documentIds
        if (mutable.value.binding != binding || mutable.value.attachmentEpoch != epoch) pending = null
        if (ids.size !in 1..128 || ids.distinct().size != ids.size || document.documentId !in ids) {
            pending = null
            mutable.value = DocumentNavigationState(active = mutable.value.active,
                issue = "The complete document list is unavailable or exceeds 128 documents.")
            return
        }
        mutable.value = DocumentNavigationState(mutable.value.active, binding, epoch, ids.toList(),
            document.title, busy = pending != null)
    }

    /** Call immediately on an observed project change, before a render refresh.
     * Rebinding the current snapshot is required before another request. */
    public fun invalidate() {
        pending = null
        mutable.value = mutable.value.copy(binding = null, busy = false,
            issue = "The document list changed. Waiting for the current view.")
    }

    public fun active(value: Boolean) { mutable.value = mutable.value.copy(active = value) }

    public fun request(expected: WorkflowBinding, epoch: Long, documentId: String): DocumentNavigationRequest? {
        val current = mutable.value
        val binding = current.binding ?: return null
        if (binding != expected || current.attachmentEpoch != epoch || !current.active || !current.ready ||
            documentId !in current.documentIds || documentId == binding.documentId) return null
        val request = DocumentNavigationRequest(binding, epoch, documentId)
        pending = request
        mutable.value = current.copy(busy = true, issue = null)
        return request
    }

    public fun matches(request: DocumentNavigationRequest, document: DocumentSnapshot, epoch: Long): Boolean {
        val current = mutable.value
        val revision = document.render.revision
        return pending === request && current.binding == request.binding && current.attachmentEpoch == epoch &&
            request.attachmentEpoch == epoch && request.binding == WorkflowBinding(revision.projectId,
            document.documentId, revision.hostSeq, revision.stateHash) && request.documentId in revision.documentIds
    }

    public fun finish(request: DocumentNavigationRequest, issue: String? = null) {
        if (pending !== request) return
        pending = null
        mutable.value = mutable.value.copy(busy = false, issue = issue)
    }

    /** Invalidates an in-flight load. The editor still joins its own producer. */
    public fun cancel() {
        pending = null
        mutable.value = mutable.value.copy(busy = false)
    }

    public fun detach() {
        pending = null
        mutable.value = DocumentNavigationState()
    }
}

/** Invoke inside the editor's existing serialized task. This only fetches a
 * candidate; the caller owns cancellation/settlement and final root adoption.
 * Page/render preparation remains on the established platform paths. */
public suspend fun loadNavigationTarget(project: WorkbenchProject, request: DocumentNavigationRequest): DocumentSnapshot {
    fun ProjectInfo.matches(): Boolean = projectId == request.binding.projectId && hostSeq == request.binding.hostSeq &&
        stateHash == request.binding.stateHash && request.binding.documentId in documentIds && request.documentId in documentIds
    currentCoroutineContext().ensureActive()
    val before = project.info()
    if (!before.matches()) throw WorkflowFailure(WorkflowFailureKind.Stale)
    val target = project.document(request.documentId)
    if (target.documentId != request.documentId || !target.render.revision.matches() || !project.info().matches()) {
        throw WorkflowFailure(WorkflowFailureKind.Stale)
    }
    currentCoroutineContext().ensureActive()
    return target
}
