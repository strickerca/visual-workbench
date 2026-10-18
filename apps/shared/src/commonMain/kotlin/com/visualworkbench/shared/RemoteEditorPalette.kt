package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex

public data class RemoteEditorPaletteState(
    public val target:RemoteTargetState?=null,
    public val busy:Boolean=false,
    public val closed:Boolean=false,
    public val message:String?=null,
    public val toolId:String?=null,
    public val settingsDigest:String?=null,
    public val compatibility:List<RemoteToolCompatibility> = emptyList(),
    /** UI observation fence, not native grant authority. */
    public val targetRevision:ULong=0uL,
    /** A partial input receipt blocks this input session until a new session. */
    public val sealedBinding:RemoteTargetBinding?=null,
)

/** UI ownership only. Native dispatch owns the actual inference/input permit,
 * target validation and cancellation retirement. Never calls local project undo.
 * Commands are not queued or retried. A receipt proves host injection only. */
@OptIn(ExperimentalCoroutinesApi::class, DelicateCoroutinesApi::class)
public class RemoteEditorPalette(
    private val dispatcher:RemoteCommandDispatcher,
    parentScope:CoroutineScope,
) {
    private val owner=SupervisorJob(parentScope.coroutineContext[Job])
    private val scope=CoroutineScope(parentScope.coroutineContext.minusKey(Job)+owner)
    private val admission=Mutex()
    private val mutable=MutableStateFlow(RemoteEditorPaletteState())
    public val state:StateFlow<RemoteEditorPaletteState> = mutable.asStateFlow()
    init {
        scope.launch {
            dispatcher.target.collect { next ->
                mutable.update { old ->
                    if(old.closed)old else {
                        val same=sameCommandSession(old.target,next)
                        old.copy(target=next,toolId=if(same)old.toolId else null,
                            settingsDigest=if(same)old.settingsDigest else null,
                            compatibility=if(same)old.compatibility else emptyList(),
                            targetRevision=if(same)old.targetRevision else old.targetRevision+1uL,
                            // A null observation is not evidence of a new input session.
                            sealedBinding=old.sealedBinding?.takeIf{next==null||sameInputSession(next.binding,it)},
                            message=if(same&&next?.grantActive==true)old.message else null)
                    }
                }
            }
        }
    }
    public fun command(action:RemoteEditorAction):Boolean {
        if(!admission.tryLock())return false
        val snapshot=mutable.value
        val target=snapshot.target
        if(target==null||snapshot.closed||snapshot.busy||dispatcher.target.value!=target||snapshot.sealedBinding?.let{sameInputSession(it,target.binding)}==true||!remoteEditorActionEnabled(target,action)){
            admission.unlock();return false
        }
        mutable.update{it.copy(busy=true,message=null)}
        // ATOMIC starts finally even when the owner is cancelled immediately.
        scope.launch(start=CoroutineStart.ATOMIC) {
            try {
                ensureActive()
                // Admission may precede execution on a queued UI dispatcher.
                // A changed producer observation is not a queued command permit.
                if(mutable.value.closed||mutable.value.targetRevision!=snapshot.targetRevision||
                    dispatcher.target.value!=target)return@launch
                val receipt=dispatcher.dispatch(target.binding,action)
                mutable.update { current ->
                    val latest=dispatcher.target.value
                    val sameTarget=current.target?.binding==target.binding&&current.target.identity==target.identity&&
                        sameCommandSession(target,latest)
                    val validReceipt=receipt.binding==target.binding&&receipt.action==action&&
                        (receipt.status==RemoteCommandStatus.Injected||(receipt.inputSequence==0uL&&receipt.acceptedQpc100ns==0uL))
                    when {
                        current.closed->current
                        // Partial input is session safety information even when
                        // the grant was revoked/restored or its geometry changed.
                        // It never becomes an editor-effect success observation.
                        validReceipt&&receipt.status==RemoteCommandStatus.SealedPartial&&
                            (current.target==null||sameInputSession(current.target.binding,target.binding))->
                            current.copy(sealedBinding=target.binding,message=if(current.target==null)null else
                                "Input was partially accepted. Control is sealed; check the editor before granting again.")
                        !sameTarget||current.targetRevision!=snapshot.targetRevision->current
                        !validReceipt->current.copy(message="Remote command receipt was refused.")
                        else->current.copy(message=when(receipt.status){
                            RemoteCommandStatus.Injected->if(!remoteEditorActionEnabled(latest?:current.target,action))null else if(receipt.inputSequence!=0uL&&receipt.acceptedQpc100ns!=0uL)"Input sent to ${target.identity.editor.name}; editor effect is unverified." else "Remote command receipt was refused."
                            RemoteCommandStatus.Refused->if(receipt.reason=="input_busy"||receipt.reason=="Backpressure")"Finish the stroke, then retry the editor command." else "${target.identity.editor.name} command refused; check the current grant and canvas focus."
                            RemoteCommandStatus.SealedPartial->null // handled by the session fence above
                        })
                    }
                }
            } catch(cancelled:CancellationException) { throw cancelled }
            catch(_:Exception) {
                mutable.update { current ->
                    val currentTarget=current.target
                    if(!current.closed&&current.targetRevision==snapshot.targetRevision&&currentTarget?.binding==target.binding&&
                        currentTarget.identity==target.identity&&sameCommandSession(target,dispatcher.target.value)&&
                        remoteEditorActionEnabled(currentTarget,action))
                        current.copy(message="Remote command failed; no retry was sent.") else current
                }
            } finally {
                // Dispatcher cancellation must join its native operation before
                // returning. This palette retains its admission until then.
                mutable.update{it.copy(busy=false)};admission.unlock()
            }
        }
        return true
    }
    /** Imported observations never enable an action or grant control. The root
     * admission lane binds receipt bytes; this function validates scope only. */
    public fun compatibility(tool:String,settings:String,records:List<RemoteToolCompatibility>):Boolean {
        val snapshot=mutable.value
        val target=snapshot.target?:return false
        val current=target.identity
        if(snapshot.closed||dispatcher.target.value!=target||records.size>64||tool.length !in 1..128||tool.any{it.code<32||it.code==127}||settings.length!=64||settings.any{it !in '0'..'9'&&it !in 'a'..'f'})return false
        if(records.map{it.aspect}.distinct().size!=records.size||records.any{!it.validFor(current,tool,settings)})return false
        // Do not return success after another owner/observation won adoption.
        // Refuse a race; imported records are never queued or retried.
        return mutable.compareAndSet(snapshot,snapshot.copy(toolId=tool,settingsDigest=settings,compatibility=records.toList()))
    }
    public suspend fun close():Unit = withContext(NonCancellable) {
        mutable.update{it.copy(closed=true,target=null,message=null,toolId=null,settingsDigest=null,compatibility=emptyList())}
        owner.cancelAndJoin()
    }
}

/** Presentation and command admission share one reason, so a disabled route is
 * visible on both platforms. Native dispatch still owns all actual authority. */
public fun remoteEditorActionUnavailableReason(target:RemoteTargetState?,action:RemoteEditorAction):String? {
    remoteEditorPaletteReason(target)?.let{return it}
    target?:return "Select a remote editor on the PC."
    if(action !in remoteEditorActions(target.identity.editor))return "This command is unavailable for the selected editor."
    if(target.selectedCanvasProof!=null&&action !in essentialEditorActions)return "This command requires verified canvas keyboard focus."
    if(action !in target.verifiedActions)return "This route is unavailable for the current editor, tool and settings."
    return null
}
public fun remoteEditorActionEnabled(target:RemoteTargetState,action:RemoteEditorAction):Boolean =
    remoteEditorActionUnavailableReason(target,action)==null

private val essentialEditorActions:Set<RemoteEditorAction> = setOf(
    RemoteEditorAction.Undo,RemoteEditorAction.Redo,RemoteEditorAction.SelectFreehandBrush,
    RemoteEditorAction.ToggleEraserMode,RemoteEditorAction.SelectEraser,
    RemoteEditorAction.BrushSmaller,RemoteEditorAction.BrushLarger,
    RemoteEditorAction.ZoomIn,RemoteEditorAction.ZoomOut,RemoteEditorAction.FitCanvas,
)
private fun RemoteCanvasIdentityProof.validFor(target:RemoteTargetState):Boolean {
    val rect=canvasRectHost
    val className=when(this){is RemoteCanvasFocusProof->focusedClassName;is RemoteSelectedCanvasProof->selectedClassName}
    val controlType=when(this){is RemoteCanvasFocusProof->focusedControlType;is RemoteSelectedCanvasProof->selectedControlType}
    return shortcutProfileDigest==target.identity.shortcutProfileDigest&&targetToken==target.binding.targetToken&&
        geometryRevision==target.binding.geometryRevision&&sampledQpc100ns!=0uL&&processId!=0u&&
        provider.length in 1..128&&provider.none{it.isISOControl()}&&
        className.length in 1..256&&className.none{it.isISOControl()}&&controlType in 50000..50100&&
        runtimeIdHash.length==64&&runtimeIdHash.all{it in '0'..'9'||it in 'a'..'f'}&&
        rect.width in 1u..32768u&&rect.height in 1u..32768u&&
        rect.x.toLong()+rect.width.toLong()<=Int.MAX_VALUE&&rect.y.toLong()+rect.height.toLong()<=Int.MAX_VALUE
}
public fun remoteEditorPaletteReason(target:RemoteTargetState?):String? = when {
    target==null->"Select a remote editor on the PC."
    !target.identity.valid()||target.identity.editor==RemoteEditorKind.Unknown->"The selected editor identity is not admitted."
    !target.grantActive->"The PC must grant control before remote commands."
    target.commandBusy->"Finishing the editor command; drawing resumes when the canvas is checked."
    target.pauseReason!=null->"Remote control is paused. Revalidate the selected editor on the PC."
    target.focusProof==null&&target.selectedCanvasProof==null->"Focus the verified editor canvas on the PC."
    target.identity.shortcutProfileDigest==null->"Shortcuts for this exact editor version have not been verified."
    (target.focusProof==null)==(target.selectedCanvasProof==null)->"The current editor canvas proof is invalid. Revalidate on the PC."
    (target.focusProof?:target.selectedCanvasProof)?.validFor(target)!=true->"The editor canvas proof is stale or invalid. Revalidate on the PC."
    target.verifiedActions.none{it in remoteEditorActions(target.identity.editor)&&
        (target.selectedCanvasProof==null||it in essentialEditorActions)}->"No verified command route is available for the current editor, tool and settings."
    else->null
}
/** The palette may retain a local partial-input fence while native state catches
 * up. Dropping an observation or refreshing its proof cannot clear that fence. */
public fun remoteEditorPaletteStateReason(state:RemoteEditorPaletteState):String? = when {
    state.closed->"Remote editor controls are closed."
    state.target!=null&&state.sealedBinding?.let{sameInputSession(it,state.target.binding)}==true->"Input was partially accepted. Control is sealed; grant a new input session on the PC."
    else->remoteEditorPaletteReason(state.target)
}
private fun sameCommandSession(before:RemoteTargetState?,after:RemoteTargetState?):Boolean {
    if(before==null||after==null)return before==after
    val previous=before.focusProof?:before.selectedCanvasProof
    val next=after.focusProof?:after.selectedCanvasProof
    return before.binding==after.binding&&before.identity==after.identity&&
        before.grantActive==after.grantActive&&before.pauseReason==after.pauseReason&&
        previous?.runtimeIdHash==next?.runtimeIdHash&&previous?.processId==next?.processId
}

private fun sameInputSession(before:RemoteTargetBinding,after:RemoteTargetBinding):Boolean =
    before.connectionEpoch==after.connectionEpoch&&before.inputSessionId==after.inputSessionId
