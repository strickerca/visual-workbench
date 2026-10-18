package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Capture when drawing a panel; never substitute a newer token on a delayed
 * callback. Serial also fences repeated visits to an identical visible hash. */
@ConsistentCopyVisibility
public data class DocumentControlsToken internal constructor(public val binding: WorkflowBinding,
    public val attachmentEpoch: Long, internal val serial: Long, public val page: UInt? = null)

public data class DocumentControlsState(
    public val active: Boolean = false,
    public val loading: Boolean = false,
    public val busy: Boolean = false,
    public val snapshot: DocumentControlSnapshot? = null,
    public val drawingLayerId: String? = null,
    public val selectedObjects: Set<String> = emptySet(),
    public val selectedGroup: String? = null,
    public val retry: WorkflowPlanInfo? = null,
    public val lastReceipt: WorkflowReceipt? = null,
    public val issue: String? = null,
) {
    public val ready: Boolean get() = active && !loading && !busy && snapshot != null && retry == null
    public val mayLeave: Boolean get() = !busy && retry == null
}

/** Call on the caller's UI dispatcher. The project is borrowed. Lazy activation
 * does not create a native capability during editor initialization. The single
 * reader and mutation owner retain cancelled jobs until their producers settle.
 * detach MUST finish before its borrowed project closes or changes document. */
public class DocumentControlsInteraction(
    parent: CoroutineScope,
    private val mutations: Mutex,
    private val metadata: suspend (WorkbenchProject) -> WorkflowMetadata,
    private val mayMutate: () -> Boolean,
    private val committed: suspend (WorkflowReceipt) -> Unit,
    private val capability: (WorkbenchProject) -> WorkbenchDocumentControls,
) {
    private data class Attachment(val project: WorkbenchProject, val documentId: String, val epoch: Long, val page: UInt?)
    private data class Pending(val attachment: Attachment, val plan: WorkbenchWorkflowPlan)
    private val supervisor: Job = SupervisorJob(parent.coroutineContext[Job])
    private val scope: CoroutineScope = CoroutineScope(parent.coroutineContext + supervisor)
    private val mutable: MutableStateFlow<DocumentControlsState> = MutableStateFlow(DocumentControlsState())
    public val state: StateFlow<DocumentControlsState> = mutable
    private var attachment: Attachment? = null
    private var binding: WorkflowBinding? = null
    private var sequence: ULong = 0uL
    private var ticket: Long = 0
    private var awaitingBinding: Boolean = false
    private var queryQueue: Channel<Unit>? = null
    private var reader: Job? = null
    private var action: Job? = null
    private var pending: Pending? = null
    private var closed: Boolean = false

    public fun capture(): DocumentControlsToken? {
        val owner = attachment ?: return null
        val current = binding ?: return null
        if (closed || !supervisor.isActive || awaitingBinding) return null
        return DocumentControlsToken(current, owner.epoch, ticket, owner.page)
    }
    private fun accepts(token: DocumentControlsToken): Boolean = capture() == token

    public fun bind(project: WorkbenchProject, document: DocumentSnapshot, epoch: Long, changeSequence: ULong) {
        if (closed || !supervisor.isActive) return
        val next = Attachment(project, document.documentId, epoch, document.page)
        val revision = document.render.revision
        val fence = WorkflowBinding(revision.projectId, document.documentId, revision.hostSeq, revision.stateHash)
        val changedOwner = attachment != next
        if (!changedOwner && changeSequence < sequence) return
        if (changedOwner) {
            // Normal app switches await detach first. This additional fence
            // rejects late output even if a caller rebinds before settlement.
            action?.cancel()
            if (action == null) releasePending()
            mutable.value = DocumentControlsState(active = mutable.value.active, busy = action != null)
            attachment = next
        }
        if (changedOwner || binding != fence || sequence != changeSequence || awaitingBinding) {
            binding = fence; sequence = changeSequence; ticket++; awaitingBinding = false
            mutable.value = mutable.value.copy(snapshot = null, loading = mutable.value.active)
            requestRead()
        }
    }

    /** Fence a delayed metadata query immediately when OPS is observed, before
     * the application's slower render refresh admits the new visible snapshot. */
    public fun observe(changeSequence: ULong) {
        if (closed || changeSequence <= sequence) return
        sequence = changeSequence; ticket++; awaitingBinding = true
        mutable.value = mutable.value.copy(snapshot = null, loading = mutable.value.active)
        // bind supplies the new visible hash. Never query the previous binding.
    }

    public fun active(value: Boolean) {
        if (closed || !supervisor.isActive) return
        mutable.value = mutable.value.copy(active = value, loading = value && mutable.value.snapshot == null)
        if (value) requestRead()
    }

    public fun reload(token: DocumentControlsToken) {
        if (!accepts(token)) return
        ticket++; mutable.value = mutable.value.copy(snapshot = null, loading = mutable.value.active, issue = null)
        requestRead()
    }

    private fun requestRead() {
        if (!mutable.value.active || attachment == null || closed || !supervisor.isActive || awaitingBinding) return
        if (reader == null) {
            val queue = Channel<Unit>(Channel.CONFLATED)
            queryQueue = queue
            reader = scope.launch {
                for (ignored in queue) {
                    val owner = attachment ?: continue
                    val expected = binding ?: continue
                    val captured = ticket
                    if (!mutable.value.active || awaitingBinding) continue
                    try {
                        val result = capability(owner.project).snapshot(owner.documentId).ownedDocumentControlSnapshot()
                        if (!matches(owner, captured) || !mutable.value.active || binding != expected) continue
                        if (result.binding != expected) {
                            mutable.value = mutable.value.copy(loading = false, issue = "The document changed. Wait for its current view, then reload controls.")
                            continue
                        }
                        // A peer lock/delete must not silently redirect the
                        // next pen stroke onto a different layer.
                        val layer = mutable.value.drawingLayerId ?: result.layers.firstOrNull(::drawable)?.layerId
                        mutable.value = mutable.value.copy(snapshot = result, loading = false, drawingLayerId = layer,
                            selectedObjects = mutable.value.selectedObjects.intersect(result.objects.map { it.objectId }.toSet()),
                            selectedGroup = mutable.value.selectedGroup?.takeIf { id -> result.groups.any { it.groupId == id } })
                    } catch (cancel: CancellationException) {
                        currentCoroutineContext().ensureActive()
                        if (matches(owner, captured)) mutable.value = mutable.value.copy(loading = false,
                            issue = "Loading was cancelled. Reload layers to try again.")
                    }
                    catch (error: Exception) {
                        if (matches(owner, captured)) mutable.value = mutable.value.copy(loading = false, issue = message(error))
                    }
                }
            }
        }
        queryQueue?.trySend(Unit)
    }

    private fun matches(owner: Attachment, captured: Long): Boolean = !closed && !awaitingBinding && attachment == owner && captured == ticket
    private fun drawable(layer: LayerControlRow): Boolean = layer.kind == "annotation" && layer.visible && !layer.locked

    public fun drawingLayer(token: DocumentControlsToken, id: String) {
        val current = mutable.value
        if (!accepts(token) || !current.ready || current.snapshot?.layers?.none { it.layerId == id && drawable(it) } != false) return
        mutable.value = current.copy(drawingLayerId = id)
    }

    public fun selectObjects(token: DocumentControlsToken, ids: Set<String>) {
        val current = mutable.value
        if (!accepts(token) || !current.ready || ids.size > 256) return
        val available = current.snapshot?.objects?.map { it.objectId }?.toSet().orEmpty()
        if (!available.containsAll(ids)) return
        mutable.value = current.copy(selectedObjects = ids.toSet(), selectedGroup = null)
    }

    public fun selectGroup(token: DocumentControlsToken, id: String?) {
        val current = mutable.value
        if (!accepts(token) || !current.ready || (id != null && current.snapshot?.groups?.none { it.groupId == id } != false)) return
        mutable.value = current.copy(selectedGroup = id, selectedObjects = emptySet())
    }

    public fun targets(): DocumentControlTargets? = mutable.value.selectedGroup?.let { DocumentControlTargets.Group(it) }
        ?: mutable.value.selectedObjects.takeIf { it.isNotEmpty() }?.let { DocumentControlTargets.Objects(it.toList().sorted()) }

    public fun execute(token: DocumentControlsToken, command: DocumentControlCommand): Boolean {
        val current = mutable.value
        val owner = attachment ?: return false
        val snapshot = current.snapshot ?: return false
        if (!accepts(token) || !current.ready || action != null || pending != null || !mayMutate()) return false
        val owned = try { command.ownedDocumentControl() } catch (error: Exception) {
            mutable.value = current.copy(issue = message(error)); return false
        }
        val captured = ticket
        return launch(owner) {
            mutations.withLock {
                if (!matches(owner, captured) || binding != snapshot.binding || !mayMutate()) throw WorkflowFailure(WorkflowFailureKind.Stale)
                val author = metadata(owner.project)
                currentCoroutineContext().ensureActive()
                if (!matches(owner, captured) || !mayMutate()) throw WorkflowFailure(WorkflowFailureKind.Stale)
                val plan = capability(owner.project).prepare(snapshot.binding, author, owned)
                // Factory settlement owns an undelivered plan; once returned,
                // this controller owns it before any cancellable operation.
                pending = Pending(owner, plan)
                mutable.value = mutable.value.copy(retry = plan.describe())
                currentCoroutineContext().ensureActive()
                if (!matches(owner, captured)) throw WorkflowFailure(WorkflowFailureKind.Stale)
                commitPending(owner)
            }
        }
    }

    /** A retry never constructs a new transaction/ID or changes the expected
     * binding. The native plan first resolves an exact accepted/pending retry. */
    public fun retry(token: DocumentControlsToken): Boolean {
        val value = pending ?: return false
        if (!accepts(token) || action != null || attachment != value.attachment || !mayMutate()) return false
        return launch(value.attachment) { mutations.withLock {
            if (attachment != value.attachment || !mayMutate()) throw WorkflowFailure(WorkflowFailureKind.Stale)
            commitPending(value.attachment)
        } }
    }

    private suspend fun commitPending(owner: Attachment) {
        val value = pending ?: throw WorkflowFailure(WorkflowFailureKind.Missing)
        if (value.attachment != owner) throw WorkflowFailure(WorkflowFailureKind.Detached)
        val receipt = value.plan.commit()
        // Durable bookkeeping precedes refresh or other fallible callbacks.
        pending = null
        if (attachment == owner) mutable.value = mutable.value.copy(lastReceipt = receipt, retry = null)
        value.plan.close()
        if (attachment == owner) committed(receipt)
    }

    private fun launch(owner: Attachment, block: suspend () -> Unit): Boolean {
        if (closed || !supervisor.isActive || action != null) return false
        mutable.value = mutable.value.copy(busy = true, issue = null)
        lateinit var job: Job
        job = scope.launch(start = CoroutineStart.LAZY) {
            try { block() }
            catch (cancel: CancellationException) {
                if (attachment == owner) mutable.value = mutable.value.copy(issue = if (pending != null)
                    "Cancellation settled. Retry the retained edit to learn whether it was saved, or discard this local retry handle."
                    else "The action was cancelled.")
                throw cancel
            } catch (error: Exception) {
                if (attachment == owner) mutable.value = mutable.value.copy(issue = message(error))
            } finally {
                if (attachment != owner) releasePending()
            }
        }
        action = job
        job.invokeOnCompletion {
            if (action === job) {
                action = null
                mutable.value = mutable.value.copy(busy = false)
                if (attachment == owner) requestRead()
            }
        }
        job.start()
        return true
    }

    public fun cancel(token: DocumentControlsToken): Unit { if (accepts(token)) action?.cancel() }
    public fun discardRetry(token: DocumentControlsToken): Boolean {
        if (!accepts(token) || action != null) return false
        releasePending()
        mutable.value = mutable.value.copy(issue = "Retry handle discarded. A previously saved edit remains in history; reload to inspect it.")
        reload(token)
        return true
    }
    private fun releasePending() {
        val value = pending; pending = null
        value?.plan?.close()
        mutable.value = mutable.value.copy(retry = null)
    }

    /** Parent shutdown signals accepted producers before waiting for its edit
     * gate. Ownership stays here until detach joins and releases exact plans. */
    public fun beginDeparture(): Unit { action?.cancel(); reader?.cancel() }

    public suspend fun detach(): Unit = withContext(NonCancellable) {
        attachment = null; binding = null; sequence = 0uL; ticket++; awaitingBinding = false
        val current = action; val reading = reader
        current?.cancel(); reading?.cancel(); queryQueue?.close(); queryQueue = null
        current?.join(); reading?.join()
        action = null; reader = null; releasePending()
        mutable.value = DocumentControlsState(active = mutable.value.active)
    }
    public suspend fun close(): Unit = withContext(NonCancellable) {
        if (!closed) { closed = true; detach(); supervisor.cancelAndJoin() }
    }

    private fun message(error: Exception): String = when ((error as? WorkflowFailure)?.kind) {
        WorkflowFailureKind.Stale -> "The document changed. Reload these controls and repeat the action."
        WorkflowFailureKind.Locked -> "Unlock the object and its layer before changing it."
        WorkflowFailureKind.Limit, WorkflowFailureKind.Memory -> "This complete operation exceeds its work or memory budget. Nothing was truncated."
        WorkflowFailureKind.Reconciliation -> "This layer order needs reconciliation. Choose a different position."
        WorkflowFailureKind.Missing -> "An operation target is no longer available. Reload these controls."
        WorkflowFailureKind.Invalid -> "These values or targets cannot be used for this operation."
        WorkflowFailureKind.Backpressure -> "The project is busy. Wait, then retry."
        WorkflowFailureKind.Closed, WorkflowFailureKind.Detached -> "This project is no longer open."
        else -> "The action could not finish. Any retained retry uses the original edit exactly."
    }
}
