package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlin.coroutines.*
import kotlin.test.*

class RemoteEditorPaletteTest {
    private fun fixture():PaletteFixture=PaletteFixture()
    @Test fun native_command_barrier_disables_palette_without_revoking_pen_grant(){
        val f=fixture();val current=f.dispatcher.target.value!!
        f.dispatcher.target.value=current.copy(commandBusy=true)
        assertFalse(f.palette.command(RemoteEditorAction.Undo));assertTrue(f.dispatcher.target.value!!.grantActive)
        assertTrue(remoteEditorPaletteReason(f.dispatcher.target.value)!!.contains("Finishing"))
        f.dispatcher.target.value=current.copy(commandBusy=false,focusProof=null,verifiedActions=emptySet())
        assertFalse(f.palette.command(RemoteEditorAction.Undo));assertTrue(f.dispatcher.target.value!!.grantActive);f.close()
    }
    @Test fun input_busy_refusal_asks_for_retry_after_release_without_resending(){
        val f=fixture();f.dispatcher.receiptTransform={it.copy(status=RemoteCommandStatus.Refused,inputSequence=0uL,acceptedQpc100ns=0uL,reason="input_busy")}
        assertTrue(f.palette.command(RemoteEditorAction.Undo));assertEquals(1,f.dispatcher.calls.size)
        assertEquals("Finish the stroke, then retry the editor command.",f.palette.state.value.message);f.close()
    }
    @Test fun stale_grant_geometry_and_missing_focus_refuse_before_dispatch(){
        val f=fixture();val initial=f.dispatcher.target.value!!
        for(target in listOf(initial.copy(grantActive=false),initial.copy(focusProof=null),initial.copy(binding=initial.binding.copy(geometryRevision=2u)),initial.copy(verifiedActions=emptySet()))){f.dispatcher.target.value=target;assertFalse(f.palette.command(RemoteEditorAction.Undo))}
        assertTrue(f.dispatcher.calls.isEmpty());f.close()
    }
    @Test fun no_queue_or_retry_while_a_remote_operation_is_pending(){
        val f=fixture();f.dispatcher.gate=CompletableDeferred();assertTrue(f.palette.command(RemoteEditorAction.Undo));assertFalse(f.palette.command(RemoteEditorAction.Redo));assertTrue(f.palette.state.value.busy);assertEquals(1,f.dispatcher.calls.size)
        f.dispatcher.gate!!.complete(Unit);assertFalse(f.palette.state.value.busy);assertTrue(f.palette.state.value.message!!.contains("effect is unverified"));f.close()
    }
    @Test fun old_epoch_receipt_cannot_reappear_in_reconnected_palette(){
        val f=fixture();f.dispatcher.gate=CompletableDeferred();f.palette.command(RemoteEditorAction.Undo)
        val original=f.dispatcher.target.value!!;f.dispatcher.target.value=original.copy(binding=original.binding.copy(connectionEpoch=2uL))
        f.dispatcher.gate!!.complete(Unit);assertNull(f.palette.state.value.message);assertFalse(f.palette.state.value.busy);f.close()
    }
    @Test fun revoked_grant_does_not_adopt_delayed_success_receipt(){
        val f=fixture();f.dispatcher.gate=CompletableDeferred();f.palette.command(RemoteEditorAction.Undo)
        f.dispatcher.target.value=f.dispatcher.target.value!!.copy(grantActive=false);f.dispatcher.gate!!.complete(Unit)
        assertNull(f.palette.state.value.message);assertFalse(f.palette.state.value.busy);f.close()
    }
    @Test fun mismatched_receipt_and_zero_success_sequence_are_refused(){
        val f=fixture();f.dispatcher.receiptTransform={it.copy(inputSequence=0uL)};f.palette.command(RemoteEditorAction.Undo);assertEquals("Remote command receipt was refused.",f.palette.state.value.message)
        f.dispatcher.receiptTransform={it.copy(action=RemoteEditorAction.Redo)};f.palette.command(RemoteEditorAction.Undo);assertEquals("Remote command receipt was refused.",f.palette.state.value.message);f.close()
    }
    @Test fun partial_batch_is_reported_without_a_success_claim_or_retry(){
        val f=fixture();f.dispatcher.receiptTransform={it.copy(status=RemoteCommandStatus.SealedPartial,inputSequence=0uL,acceptedQpc100ns=0uL)};f.palette.command(RemoteEditorAction.Undo)
        assertTrue(f.palette.state.value.message!!.contains("sealed"));assertEquals(1,f.dispatcher.calls.size);f.close()
    }
    @Test fun close_waits_for_cancelled_dispatcher_retirement(){
        val f=fixture();f.dispatcher.gate=CompletableDeferred();f.dispatcher.retirement=CompletableDeferred();f.palette.command(RemoteEditorAction.Undo)
        var closed=false;val close=f.scope.launch{f.palette.close();closed=true};assertFalse(closed);assertTrue(f.dispatcher.cancelled);assertFalse(f.palette.command(RemoteEditorAction.Redo));assertTrue(f.palette.state.value.busy)
        f.dispatcher.retirement!!.complete(Unit);assertTrue(closed);assertTrue(close.isCompleted);assertFalse(f.palette.state.value.busy);f.scope.cancel()
    }
    @Test fun compatibility_observations_never_enable_unverified_commands(){
        val f=fixture();f.dispatcher.target.value=f.dispatcher.target.value!!.copy(verifiedActions=emptySet());val identity=f.dispatcher.target.value!!.identity
        val record=RemoteToolCompatibility(identity,"brush","c".repeat(64),RemoteEditorInputApi.WindowsPointerInput,RemoteCompatibilityAspect.Shortcuts,RemoteCompatibilityResult.Supported,RemoteCompatibilityMethod.EditorObserved,"d".repeat(64),null)
        assertTrue(f.palette.compatibility("brush","c".repeat(64),listOf(record)));assertFalse(f.palette.command(RemoteEditorAction.Undo));assertTrue(f.dispatcher.calls.isEmpty());f.close()
    }
}
private fun paletteImmediate(block:suspend()->Unit){var completed:Result<Unit>?=null;block.startCoroutine(object:Continuation<Unit>{override val context:CoroutineContext=EmptyCoroutineContext;override fun resumeWith(result:Result<Unit>){completed=result}});checkNotNull(completed).getOrThrow()}
private class PaletteFixture {
    val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
    val dispatcher=PaletteDispatcher()
    val palette=RemoteEditorPalette(dispatcher,scope)
    fun close(){paletteImmediate{palette.close()};scope.cancel()}
}
private class PaletteDispatcher:RemoteCommandDispatcher {
    private val digest="a".repeat(64)
    private val binding=RemoteTargetBinding(1uL,"00000000-0000-4000-8000-000000000001",1uL,"target-token",1u,"00000000-0000-4000-8000-000000000002")
    private val identity=RemoteEditorIdentity(RemoteEditorKind.Krita,"krita.exe",digest,1024uL,"5.3.4.0",null,null,digest)
    override val target=MutableStateFlow<RemoteTargetState?>(RemoteTargetState(binding,identity,setOf(RemoteEditorAction.Undo,RemoteEditorAction.Redo),RemoteCanvasFocusProof("UIA",digest,10u,binding.targetToken,1u,100uL,"canvas",50025,RemotePhysicalRect(0,0,800u,600u),digest),true,null))
    val calls=mutableListOf<Pair<RemoteTargetBinding,RemoteEditorAction>>()
    var gate:CompletableDeferred<Unit>?=null
    var retirement:CompletableDeferred<Unit>?=null
    var cancelled=false
    var receiptTransform:(RemoteCommandReceipt)->RemoteCommandReceipt={it}
    override suspend fun dispatch(binding:RemoteTargetBinding,action:RemoteEditorAction):RemoteCommandReceipt {
        calls+=binding to action
        try{gate?.await()}catch(error:CancellationException){cancelled=true;withContext(NonCancellable){retirement?.await()};throw error}
        return receiptTransform(RemoteCommandReceipt(binding,action,1uL,200uL,RemoteCommandStatus.Injected,null))
    }
}
