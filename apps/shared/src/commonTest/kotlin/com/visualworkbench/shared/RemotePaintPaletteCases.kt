package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlin.coroutines.CoroutineContext

/** Same production leaves run by the aggregate test and the cheap standalone
 * compiler probe. All dispatch and observations below are synthetic, not native
 * grants or proof that an installed editor changed. */
internal object RemotePaintPaletteCases {
    fun run():Int {
        val cases=listOf(
            ::malformedProofIsVisible,
            ::unavailableRoutesNeverDispatch,
            ::revokeRestoreFencesDelayedSuccess,
            ::revokeRestoreFencesDelayedRefusal,
            ::revokeRestoreFencesDelayedFailure,
            ::partialInputWaitsForNewSession,
            ::delayedPartialSealsRestoredSession,
            ::delayedPartialSealsMissingObservation,
            ::toolbarFocusTransitionKeepsSequentialCommands,
            ::busyNeverQueuesAndCloseJoins,
            ::compatibilityShowsScopeAndMethod,
            ::invalidCompatibilityCannotBecomeSupport,
            ::queuedObservationCannotAdmitOldTarget,
            ::queuedObservationCannotAdoptOldReceipt,
            ::queuedCommandRechecksGrantBeforeDispatch,
            ::queuedObservationCannotImportOldCompatibility,
        )
        cases.forEach{it()}
        return cases.size
    }
    private fun queuedObservationCannotAdmitOldTarget() {
        val queue=PaintPaletteQueue()
        val f=PaintPaletteFixture(queue)
        try {
            queue.drain()
            val original=f.dispatcher.target.value!!
            f.dispatcher.target.value=original.copy(grantActive=false)
            check(f.palette.state.value.target?.grantActive==true)
            check(!f.palette.command(RemoteEditorAction.Undo))
            check(f.dispatcher.calls.isEmpty())
        } finally { f.scope.cancel();queue.drain() }
    }
    private fun queuedObservationCannotAdoptOldReceipt() {
        val queue=PaintPaletteQueue()
        val f=PaintPaletteFixture(queue)
        try {
            queue.drain()
            f.dispatcher.gate=CompletableDeferred()
            check(f.palette.command(RemoteEditorAction.Undo))
            queue.drain()
            val original=f.dispatcher.target.value!!
            f.dispatcher.target.value=original.copy(binding=original.binding.copy(inputSessionId="00000000-0000-4000-8000-000000000003"))
            f.dispatcher.gate!!.complete(Unit)
            // Complete the old dispatch before the queued target collector.
            queue.runLast()
            check(!f.palette.state.value.busy)
            check(f.palette.state.value.target?.binding==original.binding)
            check(f.palette.state.value.message==null)
            queue.drain()
            check(f.palette.state.value.target?.binding!=original.binding)
            check(f.dispatcher.calls.size==1)
        } finally { f.scope.cancel();queue.drain() }
    }
    private fun queuedCommandRechecksGrantBeforeDispatch() {
        val queue=PaintPaletteQueue()
        val f=PaintPaletteFixture(queue)
        try {
            queue.drain()
            check(f.palette.command(RemoteEditorAction.Undo))
            check(f.dispatcher.calls.isEmpty())
            f.dispatcher.target.value=f.dispatcher.target.value!!.copy(grantActive=false)
            // The command coroutine was queued first; the target collector is
            // still delayed. No old native dispatch may start in that window.
            queue.runFirst()
            check(f.dispatcher.calls.isEmpty())
            check(!f.palette.state.value.busy)
            queue.drain()
            check(!f.palette.command(RemoteEditorAction.Redo))
        } finally { f.scope.cancel();queue.drain() }
    }
    private fun queuedObservationCannotImportOldCompatibility() {
        val queue=PaintPaletteQueue()
        val f=PaintPaletteFixture(queue)
        try {
            queue.drain()
            val original=f.dispatcher.target.value!!
            val observed=RemoteToolCompatibility(original.identity,"brush","c".repeat(64),RemoteEditorInputApi.Mouse,
                RemoteCompatibilityAspect.Drawing,RemoteCompatibilityResult.Supported,
                RemoteCompatibilityMethod.EditorObserved,"d".repeat(64),null)
            f.dispatcher.target.value=original.copy(binding=original.binding.copy(connectionEpoch=2uL))
            check(!f.palette.compatibility(observed.toolId,observed.settingsDigest,listOf(observed)))
            check(f.palette.state.value.compatibility.isEmpty())
            queue.drain()
        } finally { f.scope.cancel();queue.drain() }
    }
    private fun malformedProofIsVisible() = fixture { f ->
        val target=f.dispatcher.target.value!!
        val proof=target.focusProof!!
        val invalid=listOf(
            target.copy(selectedCanvasProof=proof.selected()),
            target.copy(focusProof=proof.copy(geometryRevision=2u)),
            target.copy(focusProof=proof.copy(canvasRectHost=RemotePhysicalRect(0,0,0u,1u))),
            target.copy(focusProof=proof.copy(canvasRectHost=RemotePhysicalRect(Int.MAX_VALUE,0,1u,1u))),
            target.copy(focusProof=proof.copy(canvasRectHost=RemotePhysicalRect(0,0,32769u,1u))),
            target.copy(focusProof=proof.copy(focusedClassName="")),
            target.copy(focusProof=proof.copy(focusedControlType=42)),
            target.copy(focusProof=proof.copy(runtimeIdHash="not a digest")),
            target.copy(focusProof=proof.copy(provider="UIA\u0085")),
            target.copy(focusProof=proof.copy(processId=0u)),
            target.copy(focusProof=proof.copy(sampledQpc100ns=0uL)),
        )
        invalid.forEach{next->
            f.dispatcher.target.value=next
            check(remoteEditorPaletteStateReason(f.palette.state.value)!=null)
            check(remoteEditorActionUnavailableReason(next,RemoteEditorAction.Undo)!=null)
            check(!f.palette.command(RemoteEditorAction.Undo))
        }
        check(f.dispatcher.calls.isEmpty())
        val negative=target.copy(focusProof=proof.copy(canvasRectHost=RemotePhysicalRect(-100,-50,800u,600u)))
        check(remoteEditorActionEnabled(negative,RemoteEditorAction.Undo))
    }
    private fun unavailableRoutesNeverDispatch() = fixture { f ->
        val target=f.dispatcher.target.value!!
        check(remoteEditorActionUnavailableReason(target,RemoteEditorAction.ActualPixels)!=null)
        check(!f.palette.command(RemoteEditorAction.ActualPixels))
        f.dispatcher.target.value=target.copy(verifiedActions=setOf(RemoteEditorAction.Undo))
        check(remoteEditorPaletteReason(f.dispatcher.target.value)==null)
        check(remoteEditorActionUnavailableReason(f.dispatcher.target.value,RemoteEditorAction.BrushLarger)!=null)
        check(!f.palette.command(RemoteEditorAction.BrushLarger))
        check(f.dispatcher.calls.isEmpty())
    }
    private fun revokeRestoreFencesDelayedSuccess() = delayedRevoke(null,null)
    private fun revokeRestoreFencesDelayedRefusal() = delayedRevoke(RemoteCommandStatus.Refused,null)
    private fun revokeRestoreFencesDelayedFailure() = delayedRevoke(null,IllegalStateException("private provider detail"))
    private fun delayedRevoke(status:RemoteCommandStatus?,failure:Exception?) = fixture { f ->
        f.dispatcher.gate=CompletableDeferred()
        f.dispatcher.failure=failure
        f.dispatcher.status=status?:RemoteCommandStatus.Injected
        check(f.palette.command(RemoteEditorAction.Undo))
        val target=f.dispatcher.target.value!!
        val originalRevision=f.palette.state.value.targetRevision
        f.dispatcher.target.value=target.copy(grantActive=false)
        f.dispatcher.target.value=target
        check(f.palette.state.value.targetRevision>originalRevision)
        f.dispatcher.gate!!.complete(Unit)
        check(f.palette.state.value.message==null)
        check(!f.palette.state.value.busy)
        check(f.dispatcher.calls.size==1)
    }
    private fun partialInputWaitsForNewSession() = fixture { f ->
        f.dispatcher.status=RemoteCommandStatus.SealedPartial
        check(f.palette.command(RemoteEditorAction.Undo))
        val target=f.dispatcher.target.value!!
        check(remoteEditorPaletteStateReason(f.palette.state.value)?.contains("sealed")==true)
        check(!f.palette.command(RemoteEditorAction.Redo))
        f.dispatcher.target.value=target.copy(commandBusy=true)
        f.dispatcher.target.value=null
        f.dispatcher.target.value=target
        check(!f.palette.command(RemoteEditorAction.Redo))
        check(f.dispatcher.calls.size==1)
        f.dispatcher.status=RemoteCommandStatus.Injected
        f.dispatcher.target.value=target.copy(binding=target.binding.copy(inputSessionId="00000000-0000-4000-8000-000000000003"))
        check(remoteEditorPaletteStateReason(f.palette.state.value)==null)
        check(f.palette.command(RemoteEditorAction.Redo))
        check(f.dispatcher.calls.size==2)
    }
    private fun delayedPartialSealsRestoredSession() = delayedPartial(false)
    private fun delayedPartialSealsMissingObservation() = delayedPartial(true)
    private fun delayedPartial(missing:Boolean) = fixture { f ->
        f.dispatcher.gate=CompletableDeferred()
        f.dispatcher.status=RemoteCommandStatus.SealedPartial
        check(f.palette.command(RemoteEditorAction.Undo))
        val original=f.dispatcher.target.value!!
        f.dispatcher.target.value=original.copy(grantActive=false)
        f.dispatcher.target.value=if(missing)null else original
        f.dispatcher.gate!!.complete(Unit)
        check(f.palette.state.value.sealedBinding!=null)
        f.dispatcher.target.value=original.copy(binding=original.binding.copy(geometryRevision=2u),
            focusProof=original.focusProof!!.copy(geometryRevision=2u))
        check(!f.palette.command(RemoteEditorAction.Redo))
        check(remoteEditorPaletteStateReason(f.palette.state.value)?.contains("sealed")==true)
        check(f.dispatcher.calls.size==1)
    }
    private fun toolbarFocusTransitionKeepsSequentialCommands() = fixture { f ->
        f.dispatcher.gate=CompletableDeferred()
        check(f.palette.command(RemoteEditorAction.Undo))
        val target=f.dispatcher.target.value!!
        f.dispatcher.target.value=target.copy(focusProof=null,selectedCanvasProof=target.focusProof!!.selected())
        f.dispatcher.gate!!.complete(Unit)
        check(f.palette.state.value.message?.contains("effect is unverified")==true)
        check(f.palette.command(RemoteEditorAction.Redo))
        check(f.dispatcher.calls.size==2)
    }
    private fun busyNeverQueuesAndCloseJoins() = fixture { f ->
        f.dispatcher.gate=CompletableDeferred()
        f.dispatcher.retirement=CompletableDeferred()
        check(f.palette.command(RemoteEditorAction.Undo))
        check(!f.palette.command(RemoteEditorAction.Redo))
        var closed=false
        val closing=f.scope.launch{f.palette.close();closed=true}
        check(f.dispatcher.cancelled)
        check(!closed)
        check(!f.palette.command(RemoteEditorAction.Redo))
        f.dispatcher.retirement!!.complete(Unit)
        check(closed&&closing.isCompleted)
        check(f.dispatcher.calls.size==1)
    }
    private fun compatibilityShowsScopeAndMethod() = fixture { f ->
        val identity=f.dispatcher.target.value!!.identity
        val record=RemoteToolCompatibility(identity,"observed brush","c".repeat(64),RemoteEditorInputApi.WindowsPointerInput,
            RemoteCompatibilityAspect.Pressure,RemoteCompatibilityResult.Inconclusive,
            RemoteCompatibilityMethod.PhysicalOwnerGesture,"d".repeat(64),"synthetic pen device")
        check(f.palette.compatibility(record.toolId,record.settingsDigest,listOf(record)))
        val summary=remoteEditorCompatibilitySummary(f.palette.state.value)
        check(summary.any{it.contains(record.toolId)&&it.contains(record.settingsDigest)})
        check(summary.any{it.contains("Pressure: inconclusive")&&it.contains("physical gesture")})
        check(summary.any{it=="Hover: not tested"})
        f.dispatcher.target.value=f.dispatcher.target.value!!.copy(grantActive=false)
        check(f.palette.state.value.compatibility.isEmpty())
        check(remoteEditorCompatibilitySummary(f.palette.state.value).all{it.endsWith("not tested")})
    }
    private fun invalidCompatibilityCannotBecomeSupport() = fixture { f ->
        val identity=f.dispatcher.target.value!!.identity
        val record=RemoteToolCompatibility(identity,"brush","c".repeat(64),RemoteEditorInputApi.Unknown,
            RemoteCompatibilityAspect.Drawing,RemoteCompatibilityResult.Supported,
            RemoteCompatibilityMethod.EditorObserved,"d".repeat(64),null)
        check(!f.palette.compatibility(record.toolId,record.settingsDigest,listOf(record)))
        val unknown=identity.copy(editor=RemoteEditorKind.Unknown)
        check(!record.copy(identity=unknown,inputApi=RemoteEditorInputApi.Mouse).validFor(unknown,record.toolId,record.settingsDigest))
        check(remoteEditorCompatibilitySummary(f.palette.state.value).none{it.contains(": supported")})
        val selected=f.dispatcher.target.value!!
        f.dispatcher.target.value=selected.copy(verifiedActions=emptySet())
        val observed=record.copy(inputApi=RemoteEditorInputApi.Mouse)
        check(f.palette.compatibility(observed.toolId,observed.settingsDigest,listOf(observed)))
        check(!f.palette.command(RemoteEditorAction.Undo))
        check(remoteEditorCompatibilitySummary(f.palette.state.value).any{it.contains("Drawing: supported")&&it.contains("mouse input")})
        val physical=observed.copy(method=RemoteCompatibilityMethod.PhysicalOwnerGesture)
        check(f.palette.compatibility(physical.toolId,physical.settingsDigest,listOf(physical)))
        check(remoteEditorCompatibilitySummary(f.palette.state.value).none{it.contains("on null")})
    }
}
private fun RemoteCanvasFocusProof.selected():RemoteSelectedCanvasProof = RemoteSelectedCanvasProof(
    provider,runtimeIdHash,processId,targetToken,geometryRevision,sampledQpc100ns,focusedClassName,focusedControlType,canvasRectHost,shortcutProfileDigest)
private inline fun fixture(block:(PaintPaletteFixture)->Unit) {
    val fixture=PaintPaletteFixture()
    try{block(fixture)}finally{fixture.scope.launch{fixture.palette.close()};fixture.scope.cancel()}
}
private class PaintPaletteFixture(context:CoroutineContext=Dispatchers.Unconfined) {
    val scope=CoroutineScope(SupervisorJob()+context)
    val dispatcher=PaintPaletteDispatcher()
    val palette=RemoteEditorPalette(dispatcher,scope)
}
private class PaintPaletteDispatcher:RemoteCommandDispatcher {
    private val digest="a".repeat(64)
    private val binding=RemoteTargetBinding(1uL,"00000000-0000-4000-8000-000000000001",1uL,"target-token",1u,"00000000-0000-4000-8000-000000000002")
    private val identity=RemoteEditorIdentity(RemoteEditorKind.Paint,"mspaint.exe",digest,1024uL,"11.2605.81.0","Microsoft.Paint_11.2605.81.0_x64__publisher","11.2605.81.0",digest)
    override val target=MutableStateFlow<RemoteTargetState?>(RemoteTargetState(binding,identity,
        setOf(RemoteEditorAction.Undo,RemoteEditorAction.Redo,RemoteEditorAction.SelectFreehandBrush,RemoteEditorAction.SelectEraser,
            RemoteEditorAction.BrushSmaller,RemoteEditorAction.BrushLarger,RemoteEditorAction.ZoomIn,RemoteEditorAction.ZoomOut),
        RemoteCanvasFocusProof("UIA",digest,10u,binding.targetToken,1u,100uL,"canvas",50026,RemotePhysicalRect(0,0,800u,600u),digest),true,null))
    val calls=mutableListOf<Pair<RemoteTargetBinding,RemoteEditorAction>>()
    var status=RemoteCommandStatus.Injected
    var failure:Exception?=null
    var gate:CompletableDeferred<Unit>?=null
    var retirement:CompletableDeferred<Unit>?=null
    var cancelled=false
    override suspend fun dispatch(binding:RemoteTargetBinding,action:RemoteEditorAction):RemoteCommandReceipt {
        calls+=binding to action
        try{gate?.await()}catch(error:CancellationException){cancelled=true;withContext(NonCancellable){retirement?.await()};throw error}
        failure?.let{throw it}
        val injected=status==RemoteCommandStatus.Injected
        return RemoteCommandReceipt(binding,action,if(injected)1uL else 0uL,if(injected)200uL else 0uL,status,null)
    }
}
public fun main():Unit { println("Paint palette synthetic cases passed: ${RemotePaintPaletteCases.run()}") }

private class PaintPaletteQueue:CoroutineDispatcher() {
    private val queued=ArrayDeque<Runnable>()
    override fun dispatch(context:CoroutineContext,block:Runnable) { queued.addLast(block) }
    fun runFirst() { queued.removeFirst().run() }
    fun runLast() { queued.removeLast().run() }
    fun drain() { while(queued.isNotEmpty())queued.removeFirst().run() }
}
