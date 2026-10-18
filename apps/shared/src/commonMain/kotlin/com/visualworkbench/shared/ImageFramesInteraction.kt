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
public data class ImageFramesToken internal constructor(public val binding: WorkflowBinding,
    public val attachmentEpoch: Long, internal val serial: Long, public val page: UInt? = null)

public data class ImageFramesState(
    public val active: Boolean = false,
    public val loading: Boolean = false,
    public val busy: Boolean = false,
    public val snapshot: ImageFramesSnapshot? = null,
    public val form: ImageFramesForm? = null,
    public val formBinding: WorkflowBinding? = null,
    public val dirty: Boolean = false,
    public val stale: Boolean = false,
    public val retry: WorkflowPlanInfo? = null,
    public val lastReceipt: WorkflowReceipt? = null,
    public val preview: ImageFramePreview? = null,
    public val editingPreview: ImageEditingPreview? = null,
    public val previewLoading: Boolean = false,
    public val previewDraft: Boolean = true,
    public val previewAssumeSrgb: Boolean = false,
    public val previewMarked: Boolean = true,
    public val issue: String? = null,
    public val departing: Boolean = false,
) {
    public val ready: Boolean get() = active && !departing && !loading && !busy && !stale && snapshot != null && form != null &&
        snapshot.binding == formBinding && retry == null
    public val mayLeave: Boolean get() = !departing && !busy && retry == null && !dirty
    public val blocksInput: Boolean get() = active || !mayLeave
}

/** UI-dispatcher owner of private frame forms and one conflated preview reader.
 * The project/capability are borrowed and lazy. Call detach before closing or
 * replacing the project; cancellation alone is not settlement of native work.
 * All canonical edits use the caller's shared mutation mutex and workflow plans. */
public class ImageFramesInteraction(
    parent: CoroutineScope,
    private val mutations: Mutex,
    private val newId: () -> String,
    private val metadata: suspend (WorkbenchProject) -> WorkflowMetadata,
    private val mayMutate: () -> Boolean,
    private val committed: suspend (WorkflowReceipt) -> Unit,
    private val capability: (WorkbenchProject) -> WorkbenchImageFrames,
) {
    private data class Attachment(val project: WorkbenchProject, val documentId: String, val epoch: Long, val page: UInt?)
    private data class Pending(val attachment: Attachment, val plan: WorkbenchWorkflowPlan)
    private data class DraftCommand(val value: ImageFrameCommand.Set, val allocatedIds: Pair<String,String>?)
    private sealed interface Read {
        val owner: Attachment
        val binding: WorkflowBinding
        val ticket: Long
        data class Snapshot(override val owner: Attachment, override val binding: WorkflowBinding, override val ticket: Long) : Read
        data class Pixels(override val owner: Attachment, override val binding: WorkflowBinding, override val ticket: Long,
            val serial: Long, val snapshot: ImageFramesSnapshot, val command: ImageFrameCommand?,
            val region: SelectionRegion, val marked: Boolean, val assume: Boolean) : Read
        data class EditingPixels(override val owner: Attachment, override val binding: WorkflowBinding, override val ticket: Long,
            val serial: Long, val snapshot: ImageFramesSnapshot, val region: SelectionRegion, val assume: Boolean) : Read
    }

    private val supervisor: Job = SupervisorJob(parent.coroutineContext[Job])
    private val scope: CoroutineScope = CoroutineScope(parent.coroutineContext + supervisor)
    private val mutable: MutableStateFlow<ImageFramesState> = MutableStateFlow(ImageFramesState())
    public val state: StateFlow<ImageFramesState> = mutable
    private var attachment: Attachment? = null
    private var binding: WorkflowBinding? = null
    private var sequence: ULong = 0uL
    private var ticket: Long = 0
    private var previewSerial: Long = 0
    private var awaitingBinding: Boolean = false
    private var baseline: ImageFramesForm? = null
    private var draftIds: Pair<String,String>? = null
    private var queue: Channel<Read>? = null
    private var reader: Job? = null
    private var action: Job? = null
    private var pending: Pending? = null
    private var closed: Boolean = false
    private var departing: Boolean = false

    public fun capture(): ImageFramesToken? {
        val owner = attachment ?: return null
        val expected = binding ?: return null
        if (closed || departing || !supervisor.isActive || awaitingBinding) return null
        return ImageFramesToken(expected,owner.epoch,ticket,owner.page)
    }
    private fun accepts(token: ImageFramesToken): Boolean = capture() == token
    private fun matches(owner: Attachment, captured: Long): Boolean =
        !closed && !departing && !awaitingBinding && attachment == owner && ticket == captured

    public fun bind(project: WorkbenchProject, document: DocumentSnapshot, epoch: Long, changeSequence: ULong) {
        if (closed || departing || !supervisor.isActive) return
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
            mutable.value = ImageFramesState(active=mutable.value.active,busy=action != null)
            attachment = next
        }
        if (changedOwner || binding != expected || sequence != changeSequence || awaitingBinding) {
            binding = expected; sequence = changeSequence; ticket++; previewSerial++; awaitingBinding = false
            val current = mutable.value
            mutable.value = current.copy(snapshot=null,preview=null,editingPreview=null,previewLoading=false,loading=current.active,
                stale=current.form != null && (current.dirty || current.formBinding != expected || current.stale))
            requestSnapshot()
        }
    }

    /** Immediately fence results on OPS before the slower root render refresh. */
    public fun observe(changeSequence: ULong) {
        if (closed || departing || !supervisor.isActive || changeSequence <= sequence) return
        sequence = changeSequence; ticket++; previewSerial++; awaitingBinding = true
        mutable.value = mutable.value.copy(snapshot=null,preview=null,editingPreview=null,previewLoading=false,stale=true,loading=mutable.value.active)
    }

    public fun active(value: Boolean): Boolean {
        if (closed || departing || !supervisor.isActive || (!value && !mutable.value.mayLeave)) return false
        if (value != mutable.value.active) { ticket++; previewSerial++ }
        mutable.value = mutable.value.copy(active=value,loading=value && mutable.value.snapshot == null,
            preview=if (value) mutable.value.preview else null,
            editingPreview=if (value) mutable.value.editingPreview else null,previewLoading=false)
        if (value) requestSnapshot()
        return true
    }

    public fun edit(token: ImageFramesToken, form: ImageFramesForm): Boolean {
        val current = mutable.value
        if (!accepts(token) || !current.ready || action != null || pending != null || !form.bounded() ||
            selectedLocked(current)) return false
        previewSerial++
        mutable.value = current.copy(form=form,dirty=form != baseline,preview=null,editingPreview=null,previewLoading=false,issue=null)
        return true
    }

    public fun discardFormAndReload(token: ImageFramesToken): Boolean {
        if (!accepts(token) || action != null || pending != null) return false
        baseline = null; draftIds = null; ticket++; previewSerial++
        mutable.value = mutable.value.copy(form=null,formBinding=null,dirty=false,stale=false,snapshot=null,
            preview=null,editingPreview=null,previewLoading=false,loading=mutable.value.active,issue=null)
        requestSnapshot()
        return true
    }

    private fun selectedLocked(current: ImageFramesState): Boolean =
        current.snapshot?.frame?.locked == true

    private fun command(form: ImageFramesForm, snapshot: ImageFramesSnapshot): DraftCommand {
        // Injected ID generation may synchronously reenter the UI owner. Keep
        // these IDs local until the caller rechecks its original full fence.
        val existing = draftIds
        var allocated: Pair<String,String>? = null
        val value = form.command(snapshot) {
            existing ?: (newId() to newId()).also { allocated = it }
        }
        return DraftCommand(value,allocated)
    }

    private fun unchanged(token: ImageFramesToken, owner: Attachment, current: ImageFramesState, serial: Long): Boolean =
        accepts(token) && attachment == owner && mutable.value === current && previewSerial == serial

    public fun apply(token: ImageFramesToken): Boolean {
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

    public fun remove(token: ImageFramesToken): Boolean {
        val current = mutable.value
        val snapshot = current.snapshot ?: return false
        val owner = attachment ?: return false
        if (!accepts(token) || !current.ready || current.dirty || action != null || pending != null || !mayMutate()) return false
        if (!accepts(token) || attachment != owner || mutable.value !== current) return false
        val entry = snapshot.frame ?: return false
        if (entry.locked) return false
        return execute(token,owner,snapshot,ImageFrameCommand.Remove(entry.layerId,entry.objectId))
    }

    private fun execute(token: ImageFramesToken, owner: Attachment, snapshot: ImageFramesSnapshot, command: ImageFrameCommand): Boolean {
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

    public fun retry(token: ImageFramesToken): Boolean {
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
                snapshot=null,preview=null,editingPreview=null,previewLoading=false,loading=mutable.value.active)
        }
        try { value.plan.close() }
        finally { withContext(NonCancellable) { if (attachment == owner) committed(receipt) } }
    }

    public fun discardRetry(token: ImageFramesToken): Boolean {
        if (!accepts(token) || action != null) return false
        val failure = try { releasePending(); null } catch (error: Exception) { error }
        discardFormAndReload(token)
        mutable.value = mutable.value.copy(issue=if (failure == null)
            "The local retry handle was discarded. A saved edit remains in history; reload to inspect it."
            else "The retry handle could not be released. Close and reopen the project before continuing.")
        return failure == null
    }

    public fun cancel(token: ImageFramesToken): Unit { if (accepts(token)) action?.cancel() }

    private fun launch(owner: Attachment, block: suspend () -> Unit): Boolean {
        if (closed || departing || !supervisor.isActive || action != null) return false
        mutable.value = mutable.value.copy(busy=true,issue=null,preview=null,editingPreview=null,previewLoading=false); previewSerial++
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
    public fun invalidatePreview(token: ImageFramesToken) {
        if (!accepts(token)) return
        previewSerial++
        mutable.value = mutable.value.copy(preview=null,editingPreview=null,previewLoading=false)
    }

    public fun preview(token: ImageFramesToken, x: UInt, y: UInt, assumeUntaggedSrgb: Boolean,
        draft: Boolean = true, marked: Boolean = true): Boolean =
        requestPreview(token,null,x,y,assumeUntaggedSrgb,draft,marked)

    public fun preview(token: ImageFramesToken, region: SelectionRegion, assumeUntaggedSrgb: Boolean,
        draft: Boolean = true, marked: Boolean = true): Boolean =
        requestPreview(token,region,0u,0u,assumeUntaggedSrgb,draft,marked)

    private fun requestPreview(token: ImageFramesToken, requestedRegion: SelectionRegion?, x: UInt, y: UInt,
        assumeUntaggedSrgb: Boolean, draft: Boolean, marked: Boolean): Boolean {
        val current = mutable.value
        val snapshot = current.snapshot ?: return false
        val owner = attachment ?: return false
        val serial = previewSerial
        if (!accepts(token) || !current.ready || action != null || pending != null || snapshot.binding != binding) return false
        val region: SelectionRegion
        val command = try {
            // Region coordinates belong to O, including the complete private
            // frame's dimensions. Never validate these against source D size.
            val form = current.form ?: return false
            val expected = if (draft) form.settings(snapshot).takeIf { form.enabled } else snapshot.effectiveFrame(null)
            val width = expected?.outputWidth ?: snapshot.sourceWidth
            val height = expected?.outputHeight ?: snapshot.sourceHeight
            region = requestedRegion ?: imageFrameRegion(x,y,width,height)
            validateImageFrameRegion(region,width,height)
            if (draft) command(form,snapshot) else null
        } catch (error: Exception) {
            if (unchanged(token,owner,current,serial)) mutable.value = current.copy(issue=message(error))
            return false
        }
        if (!unchanged(token,owner,current,serial)) return false
        command?.allocatedIds?.let { draftIds = it }
        previewSerial++
        mutable.value = current.copy(preview=null,editingPreview=null,previewLoading=true,previewDraft=draft,
            previewMarked=marked,previewAssumeSrgb=assumeUntaggedSrgb,issue=null)
        enqueue(Read.Pixels(owner,snapshot.binding,token.serial,previewSerial,snapshot,command?.value,region,marked,assumeUntaggedSrgb))
        return true
    }

    /** Canonical source-space view retains pixels outside the saved output crop.
     * It already contains canonical annotations; a canvas must not paint them twice. */
    public fun editingView(token: ImageFramesToken, x: UInt, y: UInt, assumeUntaggedSrgb: Boolean): Boolean {
        val snapshot = mutable.value.snapshot ?: return false
        if (!accepts(token)) return false
        val region = try { imageFrameRegion(x,y,snapshot.sourceWidth,snapshot.sourceHeight) }
        catch (error: Exception) { mutable.value = mutable.value.copy(issue=message(error)); return false }
        return editingView(token,region,assumeUntaggedSrgb)
    }

    public fun editingView(token: ImageFramesToken, region: SelectionRegion, assumeUntaggedSrgb: Boolean): Boolean {
        val current = mutable.value
        val snapshot = current.snapshot ?: return false
        val owner = attachment ?: return false
        if (!accepts(token) || !current.ready || action != null || pending != null || snapshot.binding != binding) return false
        try { validateImageFrameRegion(region,snapshot.sourceWidth,snapshot.sourceHeight) }
        catch (error: Exception) { mutable.value = current.copy(issue=message(error)); return false }
        previewSerial++
        mutable.value = current.copy(preview=null,editingPreview=null,previewLoading=true,
            previewAssumeSrgb=assumeUntaggedSrgb,issue=null)
        enqueue(Read.EditingPixels(owner,snapshot.binding,token.serial,previewSerial,snapshot,region,assumeUntaggedSrgb))
        return true
    }

    private fun requestSnapshot() {
        val owner = attachment ?: return
        val expected = binding ?: return
        if (closed || departing || !supervisor.isActive || awaitingBinding || !mutable.value.active || action != null) return
        if (mutable.value.snapshot?.binding == expected && !mutable.value.loading) return
        enqueue(Read.Snapshot(owner,expected,ticket))
    }

    private fun enqueue(value: Read) {
        if (closed || departing || !supervisor.isActive) return
        if (reader == null) {
            val requests = Channel<Read>(Channel.CONFLATED); queue = requests
            reader = scope.launch {
                for (request in requests) {
                    if (!matches(request.owner,request.ticket) || request.binding != binding || !mutable.value.active) continue
                    try { when (request) {
                        is Read.Snapshot -> readSnapshot(request)
                        is Read.Pixels -> readPixels(request)
                        is Read.EditingPixels -> readEditingPixels(request)
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
            (when (request) {
                is Read.Pixels -> request.serial == previewSerial
                is Read.EditingPixels -> request.serial == previewSerial
                is Read.Snapshot -> true
            }))
            mutable.value = mutable.value.copy(loading=false,previewLoading=false,issue=issue)
    }

    private suspend fun readSnapshot(request: Read.Snapshot) {
        val result = capability(request.owner.project).snapshot(request.owner.documentId).ownedImageFramesSnapshot()
        if (!matches(request.owner,request.ticket) || request.binding != binding || !mutable.value.active) return
        if (result.binding != request.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
        var current = mutable.value.copy(snapshot=result,loading=false)
        if (!current.dirty && pending == null) {
            val form = result.form()
            baseline = form; draftIds = null
            current = current.copy(form=form,formBinding=result.binding,stale=false)
        } else current = current.copy(stale=current.stale || current.formBinding != result.binding)
        mutable.value = current
    }

    private suspend fun readPixels(request: Read.Pixels) {
        if (request.serial != previewSerial || !mutable.value.active) return
        val result = capability(request.owner.project).preview(request.binding,request.command,request.region,request.marked,request.assume)
        if (!matches(request.owner,request.ticket) || request.serial != previewSerial ||
            !mutable.value.active || binding != request.binding) return
        val owned = result.ownedImageFramePreview(request.snapshot,request.command,request.region)
        mutable.value = mutable.value.copy(preview=owned,previewLoading=false)
    }

    private suspend fun readEditingPixels(request: Read.EditingPixels) {
        if (request.serial != previewSerial || !mutable.value.active) return
        val result = capability(request.owner.project).editingView(request.binding,request.region,request.assume)
        if (!matches(request.owner,request.ticket) || request.serial != previewSerial ||
            !mutable.value.active || binding != request.binding) return
        val owned = result.ownedImageEditingPreview(request.snapshot,request.region)
        mutable.value = mutable.value.copy(editingPreview=owned,previewLoading=false)
    }

    private fun releasePending() {
        val value = pending; pending = null
        mutable.value = mutable.value.copy(retry=null)
        value?.plan?.close()
    }

    /** Fence every callback immediately, before the parent waits for the shared
     * mutation gate. Exact plans remain owned until detach joins settlement.
     * A late bind or activation cannot revive a cancelled reader before that join. */
    public fun beginDeparture(): Unit {
        if (!departing) {
            departing = true; ticket++; previewSerial++
            mutable.value = mutable.value.copy(departing=true,loading=false,preview=null,editingPreview=null,previewLoading=false)
        }
        action?.cancel(); reader?.cancel(); queue?.close()
    }

    public suspend fun detach(): Unit = withContext(NonCancellable) {
        beginDeparture()
        attachment = null; binding = null; sequence = 0uL; ticket++; previewSerial++; awaitingBinding = false
        val writing = action; val reading = reader
        writing?.cancel(); reading?.cancel(); queue?.close(); queue = null
        writing?.join(); reading?.join(); action = null; reader = null
        try { releasePending() }
        finally {
            baseline = null; draftIds = null
            mutable.value = ImageFramesState(active=mutable.value.active)
            departing = false
        }
    }

    public suspend fun close(): Unit = withContext(NonCancellable) {
        // Every caller waits, including a second disposal while the first is
        // still joining a producer that has entered native cancellation.
        closed = true
        try { detach() } finally { supervisor.cancelAndJoin() }
    }

    private fun message(error: Exception): String = when ((error as? WorkflowFailure)?.kind) {
        WorkflowFailureKind.Stale -> "The document changed. Discard private values and reload the saved frame before applying."
        WorkflowFailureKind.Locked -> "Unlock this frame and its layer in Documents and layers before editing it."
        WorkflowFailureKind.Invalid -> "Use positive integral crop and output sizes, a crop inside the original, and 0/90/180/270° rotation. A frame requires an original source grid at most 32768 per side; output is limited to 32768 per side and 50 million pixels."
        WorkflowFailureKind.Limit, WorkflowFailureKind.Memory -> "This complete preview or edit exceeds its work or memory budget. Nothing was resized or truncated."
        WorkflowFailureKind.Missing -> "The original or requested frame is no longer available. Reload the saved frame."
        WorkflowFailureKind.Backpressure -> "The project is busy. Wait, then retry."
        WorkflowFailureKind.Closed, WorkflowFailureKind.Detached -> "This project is no longer open."
        else -> "This frame or color conversion could not finish. Any retained retry uses the exact original edit."
    }
}
