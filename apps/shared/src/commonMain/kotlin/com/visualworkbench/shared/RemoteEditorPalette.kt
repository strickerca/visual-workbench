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
                        val same=old.target?.binding==next?.binding&&old.target?.identity==next?.identity
                        old.copy(target=next,toolId=if(same)old.toolId else null,
                            settingsDigest=if(same)old.settingsDigest else null,
                            compatibility=if(same)old.compatibility else emptyList(),
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
        if(target==null||snapshot.closed||snapshot.busy||!remoteEditorActionEnabled(target,action)){
            admission.unlock();return false
        }
        mutable.update{it.copy(busy=true,message=null)}
        // ATOMIC starts finally even when the owner is cancelled immediately.
        scope.launch(start=CoroutineStart.ATOMIC) {
            try {
                ensureActive()
                val receipt=dispatcher.dispatch(target.binding,action)
                mutable.update { current ->
                    if(current.closed||current.target?.binding!=target.binding||current.target.identity!=target.identity)current
                    else if(receipt.binding!=target.binding||receipt.action!=action||(receipt.status!=RemoteCommandStatus.Injected&&(receipt.inputSequence!=0uL||receipt.acceptedQpc100ns!=0uL)))current.copy(message="Remote command receipt was refused.")
                    else current.copy(message=when(receipt.status){
                        RemoteCommandStatus.Injected->if(!remoteEditorActionEnabled(current.target,action))null else if(receipt.inputSequence!=0uL&&receipt.acceptedQpc100ns!=0uL)"Input sent to ${target.identity.editor.name}; editor effect is unverified." else "Remote command receipt was refused."
                        RemoteCommandStatus.Refused->if(receipt.reason=="input_busy"||receipt.reason=="Backpressure")"Finish the stroke, then retry the editor command." else "${target.identity.editor.name} command refused; check the current grant and canvas focus."
                        RemoteCommandStatus.SealedPartial->"Input was partially accepted. Control is sealed; check the editor before granting again."
                    })
                }
            } catch(cancelled:CancellationException) { throw cancelled }
            catch(_:Exception) {
                mutable.update{current->if(current.target?.binding==target.binding&&!current.closed)current.copy(message="Remote command failed; no retry was sent.") else current}
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
        val current=mutable.value.target?.identity ?: return false
        if(mutable.value.closed||records.size>64||tool.length !in 1..128||tool.any{it.code<32||it.code==127}||settings.length!=64||settings.any{it !in '0'..'9'&&it !in 'a'..'f'})return false
        if(records.map{it.aspect}.distinct().size!=records.size||records.any{!it.validFor(current,tool,settings)})return false
        mutable.update{old->if(old.target?.identity==current&&!old.closed)old.copy(toolId=tool,settingsDigest=settings,compatibility=records.toList())else old}
        return true
    }
    public suspend fun close():Unit = withContext(NonCancellable) {
        mutable.update{it.copy(closed=true,target=null,message=null,toolId=null,settingsDigest=null,compatibility=emptyList())}
        owner.cancelAndJoin()
    }
}

public fun remoteEditorActionEnabled(target:RemoteTargetState,action:RemoteEditorAction):Boolean {
    if(!target.identity.valid()||!target.grantActive||target.commandBusy||target.pauseReason!=null||action !in remoteEditorActions(target.identity.editor)||action !in target.verifiedActions)return false
    val proof=target.focusProof ?: return false
    // This is an additional UI fence. Only native current proof can authorize
    // dispatch; an arbitrary caller-created descriptor has no authority.
    return target.identity.shortcutProfileDigest!=null&&proof.shortcutProfileDigest==target.identity.shortcutProfileDigest&&
        proof.targetToken==target.binding.targetToken&&proof.geometryRevision==target.binding.geometryRevision&&proof.sampledQpc100ns!=0uL&&
        proof.provider.length in 1..128&&proof.provider.none{it.code<32||it.code==127}&&proof.processId!=0u&&
        proof.runtimeIdHash.length==64&&proof.runtimeIdHash.all{it in '0'..'9'||it in 'a'..'f'}
}
public fun remoteEditorPaletteReason(target:RemoteTargetState?):String? = when {
    target==null->"Select a remote editor on the PC."
    !target.identity.valid()||target.identity.editor==RemoteEditorKind.Unknown->"The selected editor identity is not admitted."
    !target.grantActive->"The PC must grant control before remote commands."
    target.commandBusy->"Finishing the editor command; drawing resumes when the canvas is checked."
    target.pauseReason!=null->"Remote control is paused. Revalidate the selected editor on the PC."
    target.focusProof==null->"Focus the verified editor canvas on the PC."
    target.identity.shortcutProfileDigest==null||target.verifiedActions.isEmpty()->"Shortcuts for this exact editor version have not been verified."
    else->null
}
