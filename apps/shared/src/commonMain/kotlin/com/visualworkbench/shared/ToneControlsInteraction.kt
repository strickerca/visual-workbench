package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Capture while composing. Never replace a delayed callback's token with a
 * freshly captured one. Serial fences repeated visits to an identical hash. */
@ConsistentCopyVisibility
public data class ToneControlsToken internal constructor(public val binding: WorkflowBinding,
    public val attachmentEpoch: Long, internal val serial: Long, public val page: UInt? = null)

public data class ToneControlsState(
    public val active: Boolean = false,
    public val loading: Boolean = false,
    public val busy: Boolean = false,
    public val snapshot: ToneControlSnapshot? = null,
    public val form: ToneControlForm? = null,
    public val formBinding: WorkflowBinding? = null,
    public val dirty: Boolean = false,
    public val stale: Boolean = false,
    public val retry: WorkflowPlanInfo? = null,
    public val lastReceipt: WorkflowReceipt? = null,
    public val preview: ToneControlPreview? = null,
    public val previewLoading: Boolean = false,
    public val previewDraft: Boolean = true,
    public val previewAssumeSrgb: Boolean = false,
    public val issue: String? = null,
) {
    public val ready: Boolean get() = active && !loading && !busy && !stale && snapshot != null && form != null &&
        snapshot.binding == formBinding && retry == null
    public val mayLeave: Boolean get() = !busy && retry == null && !dirty
    public val blocksInput: Boolean get() = active || !mayLeave
}

/** UI-dispatcher owner of private tone forms and one conflated preview reader.
 * The project/capability are borrowed and lazy. Call detach before closing or
 * replacing the project; cancellation alone is not settlement of native work.
 * All canonical edits use the caller's shared mutation mutex and workflow plans. */
public class ToneControlsInteraction(
    parent: CoroutineScope,
    private val mutations: Mutex,
    private val newId: () -> String,
    private val metadata: suspend (WorkbenchProject) -> WorkflowMetadata,
    private val mayMutate: () -> Boolean,
    private val committed: suspend (WorkflowReceipt) -> Unit,
    private val capability: (WorkbenchProject) -> WorkbenchToneControls,
) {
    private data class Attachment(val project: WorkbenchProject, val documentId: String, val epoch: Long, val page: UInt?)
    private data class Pending(val attachment: Attachment, val plan: WorkbenchWorkflowPlan)
    private data class DraftCommand(val value: ToneControlCommand.Set, val allocatedIds: Pair<String,String>?)
    private sealed interface Read {
        val owner: Attachment
        val binding: WorkflowBinding
        val ticket: Long
        data class Snapshot(override val owner: Attachment, override val binding: WorkflowBinding, override val ticket: Long) : Read
        data class Pixels(override val owner: Attachment, override val binding: WorkflowBinding, override val ticket: Long,
            val serial: Long, val snapshot: ToneControlSnapshot, val command: ToneControlCommand?,
            val region: SelectionRegion, val assume: Boolean) : Read
    }

    private val supervisor: Job = SupervisorJob(parent.coroutineContext[Job])
    private val scope: CoroutineScope = CoroutineScope(parent.coroutineContext + supervisor)
    private val mutable: MutableStateFlow<ToneControlsState> = MutableStateFlow(ToneControlsState())
    public val state: StateFlow<ToneControlsState> = mutable
    private var attachment: Attachment? = null
    private var binding: WorkflowBinding? = null
    private var sequence: ULong = 0uL
    private var ticket: Long = 0
    private var previewSerial: Long = 0
    private var awaitingBinding: Boolean = false
    private var baseline: ToneControlForm? = null
    private var draftIds: Pair<String,String>? = null
    private var queue: Channel<Read>? = null
    private var reader: Job? = null
    private var action: Job? = null
    private var pending: Pending? = null
    private var closed: Boolean = false

    public fun capture(): ToneControlsToken? {
        val owner = attachment ?: return null
        val expected = binding ?: return null
        if (closed || !supervisor.isActive || awaitingBinding) return null
        return ToneControlsToken(expected,owner.epoch,ticket,owner.page)
    }
    private fun accepts(token: ToneControlsToken): Boolean = capture() == token
    private fun matches(owner: Attachment, captured: Long): Boolean =
        !closed && !awaitingBinding && attachment == owner && ticket == captured

    public fun bind(project: WorkbenchProject, document: DocumentSnapshot, epoch: Long, changeSequence: ULong) {
        if (closed || !supervisor.isActive) return
        val next = Attachment(project,document.documentId,epoch,document.page)
        val revision = document.render.revision
        val expected = WorkflowBinding(revision.projectId,document.documentId,revision.hostSeq,revision.stateHash)
        val changedOwner = attachment != next
        if (!changedOwner && changeSequence < sequence) return
        if (changedOwner) {
            // Normal switches await detach. This still fences a defensive early
            // rebind until the actual previous reader/mutation has settled.
            action?.cancel()
            if (action == null) releasePending()
            baseline = null; draftIds = null
            mutable.value = ToneControlsState(active=mutable.value.active,busy=action != null)
            attachment = next
        }
        if (changedOwner || binding != expected || sequence != changeSequence || awaitingBinding) {
            binding = expected; sequence = changeSequence; ticket++; previewSerial++; awaitingBinding = false
            val current = mutable.value
            mutable.value = current.copy(snapshot=null,preview=null,previewLoading=false,loading=current.active,
                stale=current.form != null && (current.dirty || current.formBinding != expected || current.stale))
            requestSnapshot()
        }
    }

    /** Immediately fence results on OPS before the slower root render refresh. */
    public fun observe(changeSequence: ULong) {
        if (closed || !supervisor.isActive || changeSequence <= sequence) return
        sequence = changeSequence; ticket++; previewSerial++; awaitingBinding = true
        mutable.value = mutable.value.copy(snapshot=null,preview=null,previewLoading=false,stale=true,loading=mutable.value.active)
    }

    public fun active(value: Boolean): Boolean {
        if (closed || !supervisor.isActive || (!value && !mutable.value.mayLeave)) return false
        if (value != mutable.value.active) { ticket++; previewSerial++ }
        mutable.value = mutable.value.copy(active=value,loading=value && mutable.value.snapshot == null,
            preview=if (value) mutable.value.preview else null,previewLoading=false)
        if (value) requestSnapshot()
        return true
    }

    public fun selectTone(token: ToneControlsToken, objectId: String?): Boolean {
        val current = mutable.value
        val snapshot = current.snapshot ?: return false
        if (!accepts(token) || !current.ready || current.dirty || action != null || pending != null ||
            (objectId != null && snapshot.tones.none { it.objectId == objectId })) return false
        if (current.form?.objectId == objectId) return true
        val form = snapshot.form(objectId)
        baseline = form; draftIds = null; ticket++; previewSerial++
        mutable.value = current.copy(form=form,formBinding=snapshot.binding,dirty=false,preview=null,previewLoading=false,issue=null)
        return true
    }

    public fun edit(token: ToneControlsToken, form: ToneControlForm): Boolean {
        val current = mutable.value
        if (!accepts(token) || !current.ready || action != null || pending != null || !form.bounded() ||
            form.objectId != current.form?.objectId || selectedLocked(current)) return false
        previewSerial++
        mutable.value = current.copy(form=form,dirty=form != baseline,preview=null,previewLoading=false,issue=null)
        return true
    }

    public fun discardFormAndReload(token: ToneControlsToken): Boolean {
        if (!accepts(token) || action != null || pending != null) return false
        baseline = null; draftIds = null; ticket++; previewSerial++
        mutable.value = mutable.value.copy(form=null,formBinding=null,dirty=false,stale=false,snapshot=null,
            preview=null,previewLoading=false,loading=mutable.value.active,issue=null)
        requestSnapshot()
        return true
    }

    private fun selectedLocked(current: ToneControlsState): Boolean =
        current.snapshot?.tones?.any { it.objectId == current.form?.objectId && it.locked } == true

    private fun command(form: ToneControlForm, snapshot: ToneControlSnapshot): DraftCommand {
        // Injected ID generation may synchronously reenter the UI owner. Keep
        // these IDs local until the caller rechecks its original full fence.
        val existing = draftIds
        var allocated: Pair<String,String>? = null
        val value = form.command(snapshot) {
            existing ?: (newId() to newId()).also { allocated = it }
        }
        return DraftCommand(value,allocated)
    }

    private fun unchanged(token: ToneControlsToken, owner: Attachment, current: ToneControlsState, serial: Long): Boolean =
        accepts(token) && attachment == owner && mutable.value === current && previewSerial == serial

    public fun apply(token: ToneControlsToken): Boolean {
        val current = mutable.value
        val snapshot = current.snapshot ?: return false
        val form = current.form ?: return false
        val owner = attachment ?: return false
        val serial = previewSerial
        if (!accepts(token) || !current.ready || action != null || pending != null || !mayMutate()) return false
        if (!unchanged(token,owner,current,serial)) return false
        val command = try { command(form,snapshot) } catch (error: Exception) {
            if (unchanged(token,owner,current,serial)) mutable.value = current.copy(issue=message(error))
            return false
        }
        if (!unchanged(token,owner,current,serial)) return false
        command.allocatedIds?.let { draftIds = it }
        return execute(token,owner,snapshot,command.value)
    }

    public fun remove(token: ToneControlsToken): Boolean {
        val current = mutable.value
        val snapshot = current.snapshot ?: return false
        val owner = attachment ?: return false
        if (!accepts(token) || !current.ready || current.dirty || action != null || pending != null || !mayMutate()) return false
        if (!accepts(token) || attachment != owner || mutable.value !== current) return false
        val entry = snapshot.tones.firstOrNull { it.objectId == current.form?.objectId } ?: return false
        if (entry.locked) return false
        return execute(token,owner,snapshot,ToneControlCommand.Remove(entry.layerId,entry.objectId))
    }

    private fun execute(token: ToneControlsToken, owner: Attachment, snapshot: ToneControlSnapshot, command: ToneControlCommand): Boolean {
        if (!accepts(token) || attachment != owner) return false
        val captured = token.serial
        return launch(owner) { mutations.withLock {
            if (!mayMutate() || !matches(owner,captured) || binding != snapshot.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            val author = metadata(owner.project)
            currentCoroutineContext().ensureActive()
            if (!mayMutate() || !matches(owner,captured) || binding != snapshot.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            val plan = capability(owner.project).prepare(snapshot.binding,author,command)
            // Own a delivered plan before any cancellable suspension. Only an
            // explicit retry can commit this same immutable handle again.
            pending = Pending(owner,plan)
            try { mutable.value = mutable.value.copy(retry=plan.describe()) }
            catch (error: Exception) { releasePending(); throw error }
            currentCoroutineContext().ensureActive()
            if (!matches(owner,captured)) throw WorkflowFailure(WorkflowFailureKind.Stale)
            commitPending(owner)
        } }
    }

    public fun retry(token: ToneControlsToken): Boolean {
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
        pending = null
        // Receipt bookkeeping is durable before a fallible close/root refresh.
        if (attachment == owner) {
            baseline = null; draftIds = null; ticket++; previewSerial++
            mutable.value = mutable.value.copy(lastReceipt=receipt,retry=null,form=null,formBinding=null,dirty=false,stale=false,
                snapshot=null,preview=null,previewLoading=false,loading=mutable.value.active)
        }
        try { value.plan.close() }
        finally { withContext(NonCancellable) { if (attachment == owner) committed(receipt) } }
    }

    public fun discardRetry(token: ToneControlsToken): Boolean {
        if (!accepts(token) || action != null) return false
        val failure = try { releasePending(); null } catch (error: Exception) { error }
        discardFormAndReload(token)
        mutable.value = mutable.value.copy(issue=if (failure == null)
            "The local retry handle was discarded. A saved edit remains in history; reload to inspect it."
            else "The retry handle could not be released. Close and reopen the project before continuing.")
        return failure == null
    }

    public fun cancel(token: ToneControlsToken): Unit { if (accepts(token)) action?.cancel() }

    private fun launch(owner: Attachment, block: suspend () -> Unit): Boolean {
        if (closed || !supervisor.isActive || action != null) return false
        mutable.value = mutable.value.copy(busy=true,issue=null,preview=null,previewLoading=false); previewSerial++
        lateinit var job: Job
        job = scope.launch(start=CoroutineStart.LAZY) {
            try { block() }
            catch (cancel: CancellationException) {
                if (attachment == owner) mutable.value = mutable.value.copy(issue=if (pending == null)
                    "The action was cancelled. Private values are still here."
                    else "Cancellation settled. Retry the exact retained edit to learn whether it was saved.")
                throw cancel
            } catch (error: Exception) {
                if (attachment == owner) mutable.value = mutable.value.copy(issue=message(error))
            } finally { if (attachment != owner) releasePending() }
        }
        action = job
        job.invokeOnCompletion {
            if (action === job) {
                action = null; mutable.value = mutable.value.copy(busy=false)
                requestSnapshot()
            }
        }
        job.start()
        return true
    }

    /** Also call for local preview input changes so a displayed tile cannot
     * masquerade as the result of a newly typed region or consent setting. */
    public fun invalidatePreview(token: ToneControlsToken) {
        if (!accepts(token)) return
        previewSerial++
        mutable.value = mutable.value.copy(preview=null,previewLoading=false)
    }

    public fun preview(token: ToneControlsToken, x: UInt, y: UInt, assumeUntaggedSrgb: Boolean,
        draft: Boolean = true): Boolean {
        val snapshot = mutable.value.snapshot ?: return false
        if (!accepts(token)) return false
        val region = try { snapshot.previewRegion(x,y) } catch (error: Exception) {
            mutable.value = mutable.value.copy(issue=message(error)); return false
        }
        return preview(token,region,assumeUntaggedSrgb,draft)
    }

    public fun preview(token: ToneControlsToken, region: SelectionRegion, assumeUntaggedSrgb: Boolean,
        draft: Boolean = true): Boolean {
        val current = mutable.value
        val snapshot = current.snapshot ?: return false
        val owner = attachment ?: return false
        val serial = previewSerial
        if (!accepts(token) || !current.ready || action != null || pending != null || snapshot.binding != binding) return false
        val command = try {
            snapshot.validateToneRegion(region)
            if (draft) command(current.form ?: return false,snapshot) else null
        } catch (error: Exception) {
            if (unchanged(token,owner,current,serial)) mutable.value = current.copy(issue=message(error))
            return false
        }
        if (!unchanged(token,owner,current,serial)) return false
        command?.allocatedIds?.let { draftIds = it }
        previewSerial++
        mutable.value = current.copy(preview=null,previewLoading=true,previewDraft=draft,
            previewAssumeSrgb=assumeUntaggedSrgb,issue=null)
        enqueue(Read.Pixels(owner,snapshot.binding,token.serial,previewSerial,snapshot,command?.value,region,assumeUntaggedSrgb))
        return true
    }

    private fun requestSnapshot() {
        val owner = attachment ?: return
        val expected = binding ?: return
        if (closed || !supervisor.isActive || awaitingBinding || !mutable.value.active || action != null) return
        if (mutable.value.snapshot?.binding == expected && !mutable.value.loading) return
        enqueue(Read.Snapshot(owner,expected,ticket))
    }

    private fun enqueue(value: Read) {
        if (reader == null) {
            val requests = Channel<Read>(Channel.CONFLATED); queue = requests
            reader = scope.launch {
                for (request in requests) {
                    if (!matches(request.owner,request.ticket) || request.binding != binding || !mutable.value.active) continue
                    try { when (request) {
                        is Read.Snapshot -> readSnapshot(request)
                        is Read.Pixels -> readPixels(request)
                    } } catch (cancel: CancellationException) {
                        // An operation-level cancellation must not kill the
                        // long-lived reader. Parent/disposal cancellation does.
                        currentCoroutineContext().ensureActive()
                        readFailed(request,"Loading was cancelled. Reload or request the preview again.")
                    } catch (error: Exception) { readFailed(request,message(error)) }
                }
            }
        }
        queue?.trySend(value)
    }

    private fun readFailed(request: Read, issue: String) {
        if (matches(request.owner,request.ticket) && mutable.value.active &&
            (request !is Read.Pixels || request.serial == previewSerial))
            mutable.value = mutable.value.copy(loading=false,previewLoading=false,issue=issue)
    }

    private suspend fun readSnapshot(request: Read.Snapshot) {
        val result = capability(request.owner.project).snapshot(request.owner.documentId).ownedToneControlSnapshot()
        if (!matches(request.owner,request.ticket) || request.binding != binding || !mutable.value.active) return
        if (result.binding != request.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
        var current = mutable.value.copy(snapshot=result,loading=false)
        if (!current.dirty && pending == null) {
            val selected = current.form?.objectId
            val form = result.form(selected)
            baseline = form; draftIds = null
            current = current.copy(form=form,formBinding=result.binding,stale=false)
        } else current = current.copy(stale=current.stale || current.formBinding != result.binding)
        mutable.value = current
    }

    private suspend fun readPixels(request: Read.Pixels) {
        if (request.serial != previewSerial || !mutable.value.active) return
        val result = capability(request.owner.project).preview(request.binding,request.command,request.region,request.assume)
        if (!matches(request.owner,request.ticket) || request.serial != previewSerial ||
            !mutable.value.active || binding != request.binding) return
        val owned = result.ownedToneControlPreview(request.snapshot,request.region)
        mutable.value = mutable.value.copy(preview=owned,previewLoading=false)
    }

    private fun releasePending() {
        val value = pending; pending = null
        mutable.value = mutable.value.copy(retry=null)
        value?.plan?.close()
    }

    public suspend fun detach(): Unit = withContext(NonCancellable) {
        attachment = null; binding = null; sequence = 0uL; ticket++; previewSerial++; awaitingBinding = false
        val writing = action; val reading = reader
        writing?.cancel(); reading?.cancel(); queue?.close(); queue = null
        writing?.join(); reading?.join(); action = null; reader = null
        try { releasePending() }
        finally { baseline = null; draftIds = null; mutable.value = ToneControlsState(active=mutable.value.active) }
    }

    public suspend fun close(): Unit = withContext(NonCancellable) {
        // Every caller waits, including a second disposal while the first is
        // still joining a producer that has entered native cancellation.
        closed = true
        try { detach() } finally { supervisor.cancelAndJoin() }
    }

    private fun message(error: Exception): String = when ((error as? WorkflowFailure)?.kind) {
        WorkflowFailureKind.Stale -> "The document changed. Discard private values and reload the saved tones before applying."
        WorkflowFailureKind.Locked -> "Unlock this tone and its layer in Documents and layers before editing it."
        WorkflowFailureKind.Invalid -> "Use finite brightness/contrast from −1 to 1, levels/opacity from 0 to 1, ordered levels and positive gamma."
        WorkflowFailureKind.Limit, WorkflowFailureKind.Memory -> "This complete preview or edit exceeds its work or memory budget. Nothing was resized or truncated."
        WorkflowFailureKind.Missing -> "The original or requested tone is no longer available. Reload the saved tones."
        WorkflowFailureKind.Backpressure -> "The project is busy. Wait, then retry."
        WorkflowFailureKind.Closed, WorkflowFailureKind.Detached -> "This project is no longer open."
        else -> "This tone or color conversion could not finish. Any retained retry uses the exact original edit."
    }
}
