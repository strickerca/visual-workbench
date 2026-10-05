package com.visualworkbench.shared

import com.visualworkbench.bindings.core.LiveSession
import com.visualworkbench.bindings.core.RemoteRuntimeFiles
import com.visualworkbench.bindings.core.SessionException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal interface NativeRemoteAccess { fun remoteEditHandle():WorkbenchRemoteEdit }
public fun remoteEdit(link:ProjectLink):WorkbenchRemoteEdit = (link as? NativeRemoteAccess)?.remoteEditHandle() ?: throw SessionFailure(SessionFailureKind.Invalid)
/** Called by the pinned desktop runtime only; peer/project/UI paths cannot
 * supply a helper or profile inventory. Existing locked inventory is retained. */
public fun configureRemoteEditRuntime(remote:WorkbenchRemoteEdit,videoPath:String,videoSha256:String,inputPath:String,inputSha256:String,profilePath:String?=null,profileSha256:String?=null) {
    (remote as? NativeRemoteEdit)?.configure(RemoteRuntimeFiles(videoPath,videoSha256,inputPath,inputSha256,profilePath,profileSha256)) ?: throw SessionFailure(SessionFailureKind.Invalid)
}
private val remoteOwners=ConcurrentHashMap.newKeySet<NativeRemoteEdit>()
public fun remoteEditOwnerCount():Int = remoteOwners.size
public fun assertRemoteEditRetired() {
    if(remoteOwners.isNotEmpty()||com.visualworkbench.bindings.core.remoteHelpersRetirementCount()!=0u)throw SessionFailure(SessionFailureKind.RemoteRetirementPending)
}
/** Detached UI cleanup uses a retained native owner, never a cancelled Compose
 * scope. Pending retirement keeps both the foreign Link and library fence. */
public fun retireRemoteEditLater(remote:WorkbenchRemoteEdit) {
    (remote as? NativeRemoteEdit)?.retireLater() ?: remote.seal("owner_pause")
}
private val retirementScope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
internal class NativeRemoteEdit(private val handle:LiveSession,private val linkOwner:Any):WorkbenchRemoteEdit {
    private val closing=AtomicBoolean(false)
    private val retired=AtomicBoolean(false)
    private val frameCollector=AtomicBoolean(false)
    private val retirementScheduled=AtomicBoolean(false)
    private val consumers=RemoteConsumerRegistry()
    private val closeMutex=Mutex()
    private val job=SupervisorJob()
    private val ownerScope=CoroutineScope(job+Dispatchers.Default)
    private val mutableState=MutableStateFlow(RemoteEditState(0uL,RemoteEditStatus.Disconnected,null,null,null,null,null))
    private val mutableTarget=MutableStateFlow<RemoteTargetState?>(null)
    override val state:StateFlow<RemoteEditState> = mutableState.asStateFlow()
    override val target:StateFlow<RemoteTargetState?> = mutableTarget.asStateFlow()
    private val stream=RemoteStreamCounters("state_published","state_stale","frames_emitted","frame_scope_discarded")
    private val publication=RemoteStreamPublication(mutableState.value){next->
        if(next.target?.binding!=mutableTarget.value?.binding||next.target?.grantActive!=true)consumers.invalidate()
        mutableTarget.value=next.target;mutableState.value=next
    }
    private fun publish(next:RemoteEditState){stream.increment(if(publication.offer(next))"state_published" else "state_stale")}
    internal fun streamDiagnostics():Map<String,ULong> = RemoteNativeJson.read(handle.remoteStreamDiagnostics()).mapValues{RemoteNativeJson.ulong(it.value)}+stream.snapshot()
    init {
        remoteOwners.add(this)
        ownerScope.launch {
            var sequence=0uL
            while(isActive&&!retired.get()) {
                try {val display=handle.waitRemoteDisplay(sequence);if(display.sequence>sequence){sequence=display.sequence;publish(display.common())}} catch(cancelled:CancellationException){throw cancelled}
                catch(error:Exception){
                    if(retired.get())break
                    consumers.invalidate();mutableTarget.value=null
                    mutableState.value=mutableState.value.copy(status=RemoteEditStatus.Sealed,target=null,reason=error.javaClass.simpleName)
                    // A malformed native DTO cannot leave an old visible grant.
                    try{handle.remotePause("current_state_unknown")}catch(_:SessionException){}
                    delay(50)
                }
            }
        }
    }
    private fun open(){if(closing.get())throw SessionFailure(SessionFailureKind.Closed)}
    internal fun isRetired():Boolean = retired.get()
    private suspend fun <T> native(block:suspend()->T):T = withContext(Dispatchers.Default){try{open();block()}catch(error:SessionException){throw sessionFailure(error)}}
    internal fun configure(files:RemoteRuntimeFiles){open();try{handle.remoteConfigureRuntime(files)}catch(error:SessionException){throw sessionFailure(error)}}
    override fun invalidateConsumers(){consumers.invalidate()}
    override fun seal(reason:String){
        invalidateConsumers()
        if(!retired.get())try{handle.remotePause(reason)}catch(error:SessionException){
            mutableState.value=mutableState.value.copy(status=RemoteEditStatus.Sealed,target=null,reason=error.javaClass.simpleName)
        }
    }
    internal fun retireLater(){
        seal("owner_pause")
        if(retirementScheduled.compareAndSet(false,true))retirementScope.launch {
            while(!retired.get()){
                try{close()}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){delay(500)}
            }
        }
    }
    override fun registerConsumer(owner:RemoteConsumerOwner){open();consumers.register(owner)}
    override val frames:Flow<RemoteVideoFrame> = flow {
        check(frameCollector.compareAndSet(false,true)){"Remote frame consumer already owned"}
        try {while(currentCoroutineContext().isActive&&!closing.get()) {
            val frame=try{handle.remoteFrame()}catch(error:SessionException){if(error is SessionException.Closed){delay(10);continue};throw sessionFailure(error)}
            if(frame==null){delay(5);continue}
            ownRemoteTicket(frame.ticket,{handle.remoteDiscardFrame(it)}){
                val config=RemoteNativeJson.config(frame.configJson)
                // Native take_frame follows scope/config admission. Publish a fresh
                // authoritative display snapshot before handing its ticket to UI.
                publish(handle.remoteDisplay().common())
                if(closing.get()||config.scope!=mutableState.value.scope){stream.increment("frame_scope_discarded");handle.remoteDiscardFrame(frame.ticket);return@ownRemoteTicket}
                stream.increment("frames_emitted")
                emit(RemoteVideoFrame(frame.ticket,config,frame.frameId,frame.ptsUs,frame.keyframe,frame.bytes))
            }
        }} finally{frameCollector.set(false)}
    }.flowOn(Dispatchers.Default)
    override suspend fun windows():List<RemoteWindowCandidate> = native{handle.remoteWindows().map{RemoteWindowCandidate(it.window,it.processId,it.processCreated,it.label,it.executableName)}}
    override suspend fun select(candidate:RemoteWindowCandidate){native{handle.remoteSelectWindow(candidate.window,candidate.processId,candidate.processCreated)}}
    override suspend fun grant(){native{handle.remoteGrant()}}
    override suspend fun requestControl(){native{handle.remoteRequestControl()}}
    override suspend fun pause(reason:String){seal(reason);native{handle.remotePause(reason)}}
    override suspend fun requestKeyframe(){native{handle.remoteKeyframe()}}
    override fun reservePen(binding:RemoteTargetBinding,samples:UInt):Boolean {
        open();return try{handle.remoteReservePen(RemoteNativeJson.binding(binding),samples)}catch(error:SessionException){throw sessionFailure(error)}
    }
    override suspend fun pen(binding:RemoteTargetBinding,sample:RemotePenSample):RemoteInputAdmission = native{
        val phase=when(sample.phase){RemotePenPhase.Hover->"Hover";RemotePenPhase.Down->"Down";RemotePenPhase.Move->"Move";RemotePenPhase.Up->"Up";RemotePenPhase.Leave->"Leave"}
        val json="{\"phase\":\"$phase\",\"x\":${sample.hostX},\"y\":${sample.hostY},\"pressure\":${sample.pressure},\"tilt_x\":${sample.tiltX},\"tilt_y\":${sample.tiltY},\"rotation\":${sample.rotation},\"pen_flags\":${sample.penFlags}}"
        val result=handle.remotePen(RemoteNativeJson.binding(binding),json)
        RemoteInputAdmission(RemoteNativeJson.binding(RemoteNativeJson.read(result.bindingJson)),result.inputSequence,sample.sampledLocalNanos)
    }
    override suspend fun discardFrame(ticket:ULong){withContext(Dispatchers.Default){if(!retired.get())handle.remoteDiscardFrame(ticket)}}
    override suspend fun rendered(ticket:ULong,ptsUs:Long):RemoteRenderedAcknowledgment? = native{handle.remoteRendered(ticket,ptsUs)?.let{RemoteRenderedAcknowledgment(RemoteNativeJson.binding(RemoteNativeJson.read(it.bindingJson)),it.frameId,it.ticket,it.lastInputSeqApplied)}}
    override suspend fun dispatch(binding:RemoteTargetBinding,action:RemoteEditorAction):RemoteCommandReceipt {
        val ordinal=action.ordinal.toUInt()+1u
        var ticket:ULong?=null;var receiptTaken=false;var completed=false
        return ownRemoteCommandAttempt(block=command@{
            try {
                val requestTicket=native{handle.remoteBeginCommand(RemoteNativeJson.binding(binding),ordinal)}
                ticket=requestTicket
                val started=System.nanoTime()
                while(System.nanoTime()-started<500_000_000L) {
                    currentCoroutineContext().ensureActive()
                    val result=native{handle.remoteTakeCommand(requestTicket)}
                    if(result!=null){
                        // The native take has consumed this ticket even if later DTO checks fail.
                        receiptTaken=true
                        require(result.requestTicket==requestTicket&&result.action==ordinal&&RemoteNativeJson.binding(RemoteNativeJson.read(result.bindingJson))==binding)
                        val receipt=checkedRemoteCommandReceipt(binding,action,result.status,result.inputSequence,result.acceptedQpc100ns,result.reason)
                        completed=true
                        return@command receipt
                    };delay(5)
                }
                RemoteCommandReceipt(binding,action,0uL,0uL,RemoteCommandStatus.Refused,"command_timeout")
            }catch(error:SessionFailure){
                // Before allocation there is no issued owner to retire. After allocation,
                // preserve the primary failure while the cleanup awaits actual retirement.
                if(ticket!=null)throw error
                RemoteCommandReceipt(binding,action,0uL,0uL,if(error.kind==SessionFailureKind.RemotePartialInput)RemoteCommandStatus.SealedPartial else RemoteCommandStatus.Refused,error.kind.name)
            }
        },retire={
            if(!completed)ticket?.let{cancelTicket->
                consumers.invalidate()
                retireIncompleteRemoteCommand(receiptTaken,
                    pause={handle.remotePause("owner_pause")},
                    cancel={handle.remoteCancelCommand(cancelTicket)},
                    retired={handle.remoteRetirementReady()})
            }
        })
    }
    override suspend fun close(){releaseNative{closeMutex.withLock{
        if(retired.get())return@withLock
        closing.set(true);val ownedConsumers=consumers.fence();consumers.invalidate()
        try{handle.remotePause("owner_pause")}catch(error:SessionException){if(error !is SessionException.Closed)throw sessionFailure(error)}
        for(consumer in ownedConsumers){if(!consumer.retire())throw SessionFailure(SessionFailureKind.RemoteRetirementPending);consumers.retired(consumer)}
        try{handle.remoteClose()}catch(error:SessionException){throw sessionFailure(error)}
        job.cancelAndJoin();retired.set(true);remoteOwners.remove(this@NativeRemoteEdit)
    }}}
}
private fun com.visualworkbench.bindings.core.RemoteDisplay.common():RemoteEditState {
    val scope=scopeJson?.let{RemoteNativeJson.scope(RemoteNativeJson.read(it))}
    val rectangle=targetJson?.let{RemoteNativeJson.rect(RemoteNativeJson.obj(RemoteNativeJson.read(it)["client_rect"]))}
    val binding=bindingJson?.let{RemoteNativeJson.binding(RemoteNativeJson.read(it))}
    val target=if(binding==null)null else {
        val authority=authorityJson?.let(RemoteNativeJson::read)
        val identity=authority?.let{RemoteNativeJson.obj(it["identity"])}
        val digest=authority?.get("profile_digest")?.let(RemoteNativeJson::string)
        val nativeIdentity=if(identity==null)RemoteEditorIdentity(RemoteEditorKind.Unknown,"","",0uL,null,null,null,null)else {
            val name=RemoteNativeJson.string(identity["executableName"])
            RemoteEditorIdentity(when(name.lowercase()){ "krita.exe"->RemoteEditorKind.Krita;"mspaint.exe"->RemoteEditorKind.Paint;else->RemoteEditorKind.Unknown},name,RemoteNativeJson.string(identity["executableBlake3"]),RemoteNativeJson.ulong(identity["executableBytes"]),RemoteNativeJson.optional(identity["fileVersion"]),RemoteNativeJson.optional(identity["packageFullName"]),RemoteNativeJson.optional(identity["packageVersion"]),digest)
        }
        val actions=authority?.get("verified_actions")?.let(RemoteNativeJson::array)?.map{RemoteNativeJson.uint(it).toInt().also{n->require(n in 1..RemoteEditorAction.entries.size)}-1}?.map{RemoteEditorAction.entries[it]}?.toSet()?:emptySet()
        val proof=authority?.get("focus")?.let{RemoteNativeJson.obj(it)}?.let{RemoteCanvasFocusProof("Windows UIA",RemoteNativeJson.string(it["runtime_id_hash"]),RemoteNativeJson.uint(it["process_id"]),binding.targetToken,binding.geometryRevision,RemoteNativeJson.ulong(it["sampled_qpc_100ns"]),RemoteNativeJson.string(it["class_name"]),RemoteNativeJson.int(it["control_type"]),RemoteNativeJson.rect(RemoteNativeJson.obj(it["canvas_rect"])),RemoteNativeJson.string(it["profile_digest"]))}
        RemoteTargetState(binding,nativeIdentity,actions,proof,status=="controlling",reason,commandBusy)
    }
    val kind=when(status){"unavailable"->RemoteEditStatus.Unavailable;"disconnected"->RemoteEditStatus.Disconnected;"selecting"->RemoteEditStatus.Selecting;"viewing"->RemoteEditStatus.Viewing;"pending_focus"->RemoteEditStatus.PendingFocus;"pending_grant"->RemoteEditStatus.PendingGrant;"controlling"->RemoteEditStatus.Controlling;"paused"->RemoteEditStatus.Paused;"sealed"->RemoteEditStatus.Sealed;"closing"->RemoteEditStatus.Closing;"closed"->RemoteEditStatus.Closed;else->error("Unknown native remote status")}
    return RemoteEditState(sequence,kind,scope,rectangle,target,reason,encoderDescription,destinationLabel)
}
