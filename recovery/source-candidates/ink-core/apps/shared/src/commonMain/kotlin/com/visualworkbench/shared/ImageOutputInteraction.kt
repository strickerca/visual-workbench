package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

public data class ImageOutputState(
    public val active: Boolean = false,
    public val loading: Boolean = false,
    public val busy: Boolean = false,
    public val snapshot: ImageOutputSnapshot? = null,
    public val form: ImageOutputForm? = null,
    public val formBinding: WorkflowBinding? = null,
    public val dirty: Boolean = false,
    public val stale: Boolean = false,
    public val retry: WorkflowPlanInfo? = null,
    public val lastReceipt: WorkflowReceipt? = null,
    public val preview: ImageOutputPreview? = null,
    public val previewLoading: Boolean = false,
    public val previewMarked: Boolean = true,
    public val previewAssumeSrgb: Boolean = false,
    public val issue: String? = null,
) {
    public val ready: Boolean get() = active && !loading && !busy && !stale && snapshot != null &&
        snapshot.binding == formBinding && retry == null
    public val mayLeave: Boolean get() = !busy && retry == null && !dirty
    public val blocksInput: Boolean get() = active || !mayLeave
}

/** UI-dispatcher owner for one borrowed project. Metadata is lazy until opening
 * this panel or Export. The one conflated reader owns one full-resolution tile;
 * it never starts work per recomposition or per pixel. Call detach before the
 * project/document changes, and close before the parent scope is disposed. */
public class ImageOutputInteraction(
    parent: CoroutineScope,
    private val mutations: Mutex,
    private val newId: () -> String,
    private val metadata: suspend (WorkbenchProject) -> WorkflowMetadata,
    private val mayMutate: () -> Boolean,
    private val committed: suspend (WorkflowReceipt) -> Unit,
    private val capability: (WorkbenchProject) -> WorkbenchImageOutputs = { it.imageOutputs() },
) {
    private data class Attachment(val project: WorkbenchProject, val documentId: String, val epoch: Long)
    private data class Pending(val attachment: Attachment, val plan: WorkbenchWorkflowPlan)
    private sealed interface Read {
        val owner: Attachment
        val binding: WorkflowBinding
        val ticket: Long
        data class Snapshot(override val owner: Attachment, override val binding: WorkflowBinding, override val ticket: Long) : Read
        data class Pixels(override val owner: Attachment, override val binding: WorkflowBinding, override val ticket: Long,
            val serial: Long, val snapshot: ImageOutputSnapshot, val region: SelectionRegion, val marked: Boolean, val assume: Boolean) : Read
    }
    private val supervisor: Job = SupervisorJob(parent.coroutineContext[Job])
    private val scope: CoroutineScope = CoroutineScope(parent.coroutineContext + supervisor)
    private val mutable: MutableStateFlow<ImageOutputState> = MutableStateFlow(ImageOutputState())
    public val state: StateFlow<ImageOutputState> = mutable
    private var attachment: Attachment? = null
    private var binding: WorkflowBinding? = null
    private var sequence: ULong = 0uL
    private var ticket: Long = 0
    private var previewSerial: Long = 0
    private var awaitingBinding: Boolean = false
    private var exportRequested: Boolean = false
    private var baseline: ImageOutputForm? = null
    private var queue: Channel<Read>? = null
    private var reader: Job? = null
    private var action: Job? = null
    private var pending: Pending? = null
    private var closed: Boolean = false

    public fun bind(project: WorkbenchProject, document: DocumentSnapshot, epoch: Long, changeSequence: ULong) {
        if (closed || !supervisor.isActive) return
        val next = Attachment(project,document.documentId,epoch)
        val revision = document.render.revision
        val expected = WorkflowBinding(revision.projectId,document.documentId,revision.hostSeq,revision.stateHash)
        val changedOwner = attachment != next
        if (!changedOwner && changeSequence < sequence) return
        if (changedOwner) {
            action?.cancel()
            if (action == null) releasePending()
            baseline = null; exportRequested = false
            mutable.value = ImageOutputState(active=mutable.value.active,busy=action!=null)
            attachment = next
        }
        if (changedOwner || binding != expected || sequence != changeSequence || awaitingBinding) {
            binding=expected; sequence=changeSequence; ticket++; previewSerial++; awaitingBinding=false
            mutable.value=mutable.value.copy(snapshot=null,preview=null,previewLoading=false,
                loading=mutable.value.active || exportRequested,stale=mutable.value.formBinding?.let{it!=expected} ?: false)
            requestSnapshot()
        }
    }

    /** OPS invalidates metadata and pixels immediately, before render refresh. */
    public fun observe(changeSequence: ULong) {
        if (closed || changeSequence <= sequence) return
        sequence=changeSequence; ticket++; previewSerial++; awaitingBinding=true
        mutable.value=mutable.value.copy(snapshot=null,preview=null,previewLoading=false,stale=true,
            loading=mutable.value.active || exportRequested)
    }

    public fun active(value: Boolean): Boolean {
        if (closed || !supervisor.isActive || (!value && !mutable.value.mayLeave)) return false
        mutable.value=mutable.value.copy(active=value,loading=(value || exportRequested) && mutable.value.snapshot==null,
            preview=if(value)mutable.value.preview else null,previewLoading=false)
        previewSerial++
        if(value)requestSnapshot()
        return true
    }

    /** An explicit Export action may load output metadata without opening or
     * changing a form. Missing metadata always disables request capture. */
    public fun requestExportContext() {
        if(closed || !supervisor.isActive)return
        exportRequested=true
        if(mutable.value.snapshot?.binding!=binding || awaitingBinding) {
            mutable.value=mutable.value.copy(loading=true)
            requestSnapshot()
        }
    }
    public fun endExportContext() { exportRequested=false }
    public fun exportSnapshot(document: DocumentSnapshot): ImageOutputSnapshot? = mutable.value.snapshot
        ?.takeIf { !closed && !awaitingBinding && it.binding==binding && it.matches(document) }

    public fun selectFrame(): Boolean = selectForm(null,false)
    public fun selectTone(objectId: String?): Boolean = selectForm(objectId,true)
    private fun selectForm(objectId: String?, tone: Boolean): Boolean {
        val s=mutable.value; val snapshot=s.snapshot ?: return false
        if(!s.ready || s.dirty || action!=null || pending!=null || (objectId!=null && snapshot.tones.none{it.objectId==objectId}))return false
        val value=snapshot.form(objectId,tone); baseline=value
        mutable.value=s.copy(form=value,formBinding=snapshot.binding,dirty=false,issue=null)
        return true
    }
    public fun edit(form: ImageOutputForm): Boolean {
        val s=mutable.value
        if(!s.ready || action!=null || pending!=null || !form.bounded() ||
            (form is ImageOutputForm.Frame)!=(s.form is ImageOutputForm.Frame) ||
            (form is ImageOutputForm.Tone && form.objectId!=(s.form as? ImageOutputForm.Tone)?.objectId))return false
        mutable.value=s.copy(form=form,dirty=form!=baseline,issue=null)
        return true
    }

    /** Explicitly discards only private typing. Canonical data and any uncertain
     * transaction are untouched. A retained transaction must be resolved first. */
    public fun discardFormAndReload(): Boolean {
        if(closed || !supervisor.isActive || action!=null || pending!=null)return false
        baseline=null; ticket++; previewSerial++
        mutable.value=mutable.value.copy(form=null,formBinding=null,dirty=false,stale=false,snapshot=null,
            preview=null,previewLoading=false,loading=mutable.value.active || exportRequested,issue=null)
        requestSnapshot(); return true
    }

    public fun apply(): Boolean {
        if(closed || !supervisor.isActive)return false
        val s=mutable.value; val snapshot=s.snapshot ?: return false; val form=s.form ?: return false
        if(!s.ready || action!=null || pending!=null || !mayMutate())return false
        val command=try{form.command(snapshot,newId)}catch(error:Exception){mutable.value=s.copy(issue=message(error));return false}
        return execute(snapshot,command)
    }
    public fun remove(): Boolean {
        if(closed || !supervisor.isActive)return false
        val s=mutable.value; val snapshot=s.snapshot ?: return false
        if(!s.ready || s.dirty || action!=null || pending!=null || !mayMutate())return false
        val command=when(val form=s.form){
            is ImageOutputForm.Frame -> snapshot.output?.let{if(it.locked)return false;ImageOutputCommand.RemoveFrame(it.layerId,it.objectId)}
            is ImageOutputForm.Tone -> snapshot.tones.firstOrNull{it.objectId==form.objectId}?.let{
                if(it.locked)return false;ImageOutputCommand.RemoveTone(it.layerId,it.objectId)}
            null -> null
        } ?: return false
        return execute(snapshot,command)
    }
    private fun execute(snapshot:ImageOutputSnapshot,command:ImageOutputCommand):Boolean {
        val owner=attachment ?: return false; val captured=ticket
        return launch(owner){mutations.withLock {
            if(!matches(owner,captured) || binding!=snapshot.binding || !mayMutate())throw WorkflowFailure(WorkflowFailureKind.Stale)
            val author=metadata(owner.project)
            currentCoroutineContext().ensureActive()
            if(!matches(owner,captured) || binding!=snapshot.binding || !mayMutate())throw WorkflowFailure(WorkflowFailureKind.Stale)
            val plan=capability(owner.project).prepare(snapshot.binding,author,command)
            pending=Pending(owner,plan)
            mutable.value=mutable.value.copy(retry=plan.describe())
            currentCoroutineContext().ensureActive()
            if(!matches(owner,captured))throw WorkflowFailure(WorkflowFailureKind.Stale)
            commitPending(owner)
        }}
    }
    public fun retry(): Boolean {
        val value=pending ?: return false
        if(closed || action!=null || attachment!=value.attachment || !mayMutate())return false
        return launch(value.attachment){mutations.withLock {
            if(attachment!=value.attachment || !mayMutate())throw WorkflowFailure(WorkflowFailureKind.Stale)
            commitPending(value.attachment)
        }}
    }
    private suspend fun commitPending(owner:Attachment) {
        val value=pending ?: throw WorkflowFailure(WorkflowFailureKind.Missing)
        if(value.attachment!=owner)throw WorkflowFailure(WorkflowFailureKind.Detached)
        val receipt=value.plan.commit()
        pending=null
        // Once accepted, fallible UI refresh cannot turn the edit back into an
        // uncertain retry or strand the saved private form.
        if(attachment==owner) {
            baseline=null; previewSerial++
            mutable.value=mutable.value.copy(lastReceipt=receipt,retry=null,form=null,formBinding=null,dirty=false,
                snapshot=null,preview=null,previewLoading=false,loading=mutable.value.active || exportRequested)
        }
        try { value.plan.close() }
        finally { withContext(NonCancellable) { if(attachment==owner)committed(receipt) } }
    }
    public fun discardRetry(): Boolean {
        if(closed || !supervisor.isActive || action!=null)return false
        val failure=try{releasePending();null}catch(error:Exception){error}
        discardFormAndReload()
        mutable.value=mutable.value.copy(issue=if(failure==null)"The local retry handle was discarded. A previously saved edit remains in history; reload to inspect it."
            else "The retry handle reported a release failure. The saved project is intact; close and reopen the project before continuing.")
        return failure==null
    }
    public fun cancel() { action?.cancel() }
    private fun launch(owner:Attachment,block:suspend()->Unit):Boolean {
        if(closed || !supervisor.isActive || action!=null)return false
        mutable.value=mutable.value.copy(busy=true,issue=null,preview=null,previewLoading=false); previewSerial++
        lateinit var job:Job
        job=scope.launch(start=CoroutineStart.LAZY){
            try{block()}
            catch(cancel:CancellationException){
                if(attachment==owner)mutable.value=mutable.value.copy(issue=if(pending==null)"The action was cancelled. Private values are still here."
                    else "Cancellation settled. Retry the exact retained edit to learn whether it was saved.")
                throw cancel
            }catch(error:Exception){if(attachment==owner)mutable.value=mutable.value.copy(issue=message(error))}
            finally{if(attachment!=owner)releasePending()}
        }
        action=job
        job.invokeOnCompletion{if(action===job){action=null;mutable.value=mutable.value.copy(busy=false);requestSnapshot()}}
        job.start();return true
    }

    public fun preview(x:UInt,y:UInt,marked:Boolean,assumeUntaggedSrgb:Boolean):Boolean {
        val s=mutable.value; val snapshot=s.snapshot ?: return false; val owner=attachment ?: return false
        if(!s.active || s.loading || s.busy || s.stale || pending!=null || snapshot.binding!=binding || awaitingBinding || closed || !supervisor.isActive)return false
        val region=try{imageOutputPreviewRegion(snapshot,x,y)}catch(error:Exception){mutable.value=s.copy(issue=message(error));return false}
        previewSerial++
        mutable.value=s.copy(preview=null,previewLoading=true,previewMarked=marked,previewAssumeSrgb=assumeUntaggedSrgb,issue=null)
        enqueue(Read.Pixels(owner,snapshot.binding,ticket,previewSerial,snapshot,region,marked,assumeUntaggedSrgb))
        return true
    }
    private fun requestSnapshot() {
        val owner=attachment ?: return; val expected=binding ?: return
        if(closed || !supervisor.isActive || awaitingBinding || (!mutable.value.active && !exportRequested) || action!=null)return
        if(mutable.value.snapshot?.binding==expected && !mutable.value.loading)return
        enqueue(Read.Snapshot(owner,expected,ticket))
    }
    private fun enqueue(value:Read) {
        if(reader==null) {
            val requests=Channel<Read>(Channel.CONFLATED);queue=requests
            reader=scope.launch {
                for(request in requests){
                    if(!matches(request.owner,request.ticket) || request.binding!=binding)continue
                    try{when(request){
                        is Read.Snapshot -> readSnapshot(request)
                        is Read.Pixels -> readPixels(request)
                    }}catch(cancel:CancellationException){throw cancel}
                    catch(error:Exception){if(matches(request.owner,request.ticket) &&
                        (request !is Read.Pixels || request.serial==previewSerial))mutable.value=mutable.value.copy(loading=false,previewLoading=false,issue=message(error))}
                }
            }
        }
        queue?.trySend(value)
    }
    private suspend fun readSnapshot(request:Read.Snapshot) {
        if(!mutable.value.active && !exportRequested)return
        val result=capability(request.owner.project).snapshot(request.owner.documentId)
        if(!matches(request.owner,request.ticket) || request.binding!=binding || (!mutable.value.active && !exportRequested))return
        if(result.binding!=request.binding)throw WorkflowFailure(WorkflowFailureKind.Stale)
        var s=mutable.value.copy(snapshot=result,loading=false)
        if(!s.dirty && pending==null) {
            val old=s.form as? ImageOutputForm.Tone
            val form=result.form(old?.objectId,old!=null);baseline=form
            s=s.copy(form=form,formBinding=result.binding,stale=false)
        }else s=s.copy(stale=s.formBinding!=result.binding)
        mutable.value=s
    }
    private suspend fun readPixels(request:Read.Pixels) {
        if(!mutable.value.active || request.serial!=previewSerial)return
        val result=capability(request.owner.project).preview(request.binding,request.region,request.marked,request.assume)
        if(!matches(request.owner,request.ticket) || request.serial!=previewSerial || !mutable.value.active || binding!=request.binding)return
        val expected=request.snapshot; val region=request.region
        if(result.binding!=expected.binding || result.sourceAssetId!=expected.sourceAssetId ||
            result.output!=expected.output?.takeIf{it.enabled}?.settings || result.outputWidth!=expected.outputWidth || result.outputHeight!=expected.outputHeight ||
            result.x!=region.x || result.y!=region.y || result.width!=region.width || result.height!=region.height ||
            result.rgba.size.toULong()!=region.width.toULong()*region.height.toULong()*4uL)throw WorkflowFailure(WorkflowFailureKind.Invalid)
        mutable.value=mutable.value.copy(preview=result,previewLoading=false)
    }
    private fun matches(owner:Attachment,captured:Long):Boolean = !closed && !awaitingBinding && attachment==owner && captured==ticket
    private fun releasePending(){val value=pending;pending=null;mutable.value=mutable.value.copy(retry=null);value?.plan?.close()}

    public suspend fun detach():Unit=withContext(NonCancellable){
        attachment=null;binding=null;sequence=0uL;ticket++;previewSerial++;awaitingBinding=false;exportRequested=false
        val writing=action;val reading=reader
        writing?.cancel();reading?.cancel();queue?.close();queue=null
        writing?.join();reading?.join();action=null;reader=null
        try{releasePending()}finally{baseline=null;mutable.value=ImageOutputState(active=mutable.value.active)}
    }
    public suspend fun close():Unit=withContext(NonCancellable){if(!closed){closed=true;try{detach()}finally{supervisor.cancelAndJoin()}}}
    private fun message(error:Exception):String=when((error as? WorkflowFailure)?.kind){
        WorkflowFailureKind.Stale -> "The document changed. Discard private values and reload the committed output before applying."
        WorkflowFailureKind.Locked -> "Unlock this object and its layer in Documents and layers before editing it."
        WorkflowFailureKind.Invalid -> "Use an in-bounds integer crop, positive output size, a quarter-turn and valid ordered levels."
        WorkflowFailureKind.Limit,WorkflowFailureKind.Memory -> "The complete image or edit exceeds its work or memory limit. Nothing was resized or truncated."
        WorkflowFailureKind.Missing -> "The original or requested setting is no longer available. Reload the committed output."
        WorkflowFailureKind.Backpressure -> "The project is busy. Wait, then retry."
        WorkflowFailureKind.Closed,WorkflowFailureKind.Detached -> "This project is no longer open."
        else -> "This image or color conversion could not be completed. The original is intact; any retained retry is the exact same edit."
    }
}
