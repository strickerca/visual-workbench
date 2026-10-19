package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex

/** Implemented only by the checked native adapter. Preparation is query-only;
 * confirmation owns actual input exclusion, helper joins and source validation.
 * Cancellation cannot abandon an undelivered native preparation. */
internal interface RemoteDrawingCheckCapability {
    val drawingCheckSupported:Boolean
    suspend fun prepareDrawingCheck(expected:RemoteTargetState):RemotePreparedDrawingCheck
}
internal interface RemotePreparedDrawingCheck {
    val target:RemoteTargetState
    /** One-shot native producer, reached only after this owner's confirmation.
     * Native rechecks its source/tool and actual queued-input exclusion. Any
     * admitted receipt must also remain in the native session's evidence owner. */
    suspend fun runConfirmed():RemoteCompatibilityAdmission?
    /** Joins process/Job/IO and disposes private files before returning. Failure
     * retains this same owner for retry. Restored requires actual pixel proof;
     * retirement alone never establishes the canvas state. Reads any terminal
     * state before foreign destruction; evidence remains session-owned. */
    suspend fun close():RemoteDrawingCanvasState
}
internal enum class RemoteDrawingCanvasState { Unchanged,Restored,RecoveryNeeded,Unknown }
public enum class RemoteDrawingCheckPhase { Idle,Preparing,AwaitingConfirmation,Running,CleanupPending,Complete,Closed }
public data class RemoteDrawingCheckState(
    public val phase:RemoteDrawingCheckPhase=RemoteDrawingCheckPhase.Idle,
    public val target:RemoteEditorIdentity?=null,
    public val message:String?=null,
    public val cleanupPending:Boolean=false,
    public val canvasRecoveryNeeded:Boolean=false,
)

/** UI lifetime and explicit confirmation only; it cannot manufacture evidence.
 * The palette supplies its actual command admission mutex so a prepared or
 * retiring check cannot overlap commands/imports. Native still owns authority. */
@OptIn(ExperimentalCoroutinesApi::class,DelicateCoroutinesApi::class)
internal class RemoteDrawingCheck(
    private val dispatcher:RemoteCommandDispatcher,
    parent:CoroutineScope,
    private val admission:Mutex,
    private val adopt:(RemoteCompatibilityAdmission)->Boolean,
    private val busy:(Boolean)->Unit={},
    private val seal:(RemoteTargetBinding)->Unit={},
) {
    private class Operation(val expected:RemoteTargetState) {
        val decision=CompletableDeferred<Boolean>()
        val prepared=MutableStateFlow<RemotePreparedDrawingCheck?>(null)
        val task=MutableStateFlow<Job?>(null)
        val retirement=MutableStateFlow<CompletableDeferred<Unit>?>(null)
        val permitReleased=MutableStateFlow(false)
        val cancelled=MutableStateFlow(false)
        val result=MutableStateFlow<RemoteCompatibilityAdmission?>(null)
        val confirmed=MutableStateFlow(false)
    }
    private val closed=MutableStateFlow(false)
    private val recoveryBinding=MutableStateFlow<RemoteTargetBinding?>(null)
    private val current=MutableStateFlow<Operation?>(null)
    private val lifetime=SupervisorJob(parent.coroutineContext[Job])
    private val scope=CoroutineScope(parent.coroutineContext.minusKey(Job)+lifetime)
    private val mutable=MutableStateFlow(RemoteDrawingCheckState())
    val state:StateFlow<RemoteDrawingCheckState> = mutable.asStateFlow()
    init {
        scope.launch {
            dispatcher.target.collect { target->
                current.value?.takeIf{!same(it.expected,target)}?.let {op->
                    op.cancelled.value=true;op.decision.complete(false);op.task.value?.cancel()
                }
            }
        }
    }
    private fun same(expected:RemoteTargetState,actual:RemoteTargetState?):Boolean =
        actual?.binding==expected.binding&&actual.identity==expected.identity&&actual.grantActive
    private fun recoveryBlocked(binding:RemoteTargetBinding):Boolean = recoveryBinding.value?.let {
        it.connectionEpoch==binding.connectionEpoch&&it.inputSessionId==binding.inputSessionId
    }==true

    fun prepare():Boolean {
        if(closed.value||!lifetime.isActive||!admission.tryLock())return false
        val expected=dispatcher.target.value
        val capability=dispatcher as? RemoteDrawingCheckCapability
        if(expected==null||!expected.grantActive||recoveryBlocked(expected.binding)||expected.identity.editor!=RemoteEditorKind.Paint||
            !expected.identity.valid()||capability?.drawingCheckSupported!=true) {admission.unlock();return false}
        val op=Operation(expected)
        if(!current.compareAndSet(null,op)){admission.unlock();return false}
        busy(true)
        mutable.value=RemoteDrawingCheckState(RemoteDrawingCheckPhase.Preparing,expected.identity)
        val task=scope.launch(start=CoroutineStart.ATOMIC) {
            try {
                op.task.value=currentCoroutineContext()[Job]
                ensureActive()
                // Adoption happens inside the cancellation-safe acquisition
                // bracket, before returning to a potentially cancelled caller.
                withContext(NonCancellable) {
                    val prepared=capability.prepareDrawingCheck(expected)
                    op.prepared.value=prepared
                }
                ensureActive()
                val prepared=op.prepared.value?:error("Missing prepared drawing owner")
                if(op.cancelled.value||!same(expected,dispatcher.target.value)||!same(expected,prepared.target))return@launch
                mutable.value=RemoteDrawingCheckState(RemoteDrawingCheckPhase.AwaitingConfirmation,expected.identity)
                if(!op.decision.await())return@launch
                ensureActive()
                if(op.cancelled.value||!same(expected,dispatcher.target.value))return@launch
                op.confirmed.value=true
                mutable.value=RemoteDrawingCheckState(RemoteDrawingCheckPhase.Running,expected.identity)
                val result=prepared.runConfirmed()
                ensureActive()
                if(same(expected,dispatcher.target.value)&&result!=null&&result.validFor(expected)) {
                    op.result.value=result
                }
            } catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){mutable.value=RemoteDrawingCheckState(RemoteDrawingCheckPhase.Complete,expected.identity,
                "The drawing check could not complete. Check the selected Paint canvas before drawing again.")}
            finally {withContext(NonCancellable){
                try {retire(op)}catch(_:Exception){/* same owner and permit remain retained */}
            }}
        }
        op.task.compareAndSet(null,task)
        return true
    }
    fun confirm():Boolean {
        val op=current.value?:return false
        if(closed.value||mutable.value.phase!=RemoteDrawingCheckPhase.AwaitingConfirmation||
            !same(op.expected,dispatcher.target.value))return false
        return op.decision.complete(true)
    }
    fun cancel() {current.value?.let{it.cancelled.value=true;it.decision.complete(false);it.task.value?.cancel()}}

    private suspend fun retire(op:Operation) {
        val flight=CompletableDeferred<Unit>()
        while(true) {
            val active=op.retirement.value
            if(active!=null){active.await();return}
            if(op.retirement.compareAndSet(null,flight))break
        }
        try {
            val canvas=op.prepared.value?.close()?:RemoteDrawingCanvasState.Unchanged
            op.prepared.value=null
            val recovery=canvas in setOf(RemoteDrawingCanvasState.RecoveryNeeded,RemoteDrawingCanvasState.Unknown)
            if(recovery){recoveryBinding.value=op.expected.binding;seal(op.expected.binding)}
            val result=op.result.value
            op.result.value=null
            val adopted=if(!closed.value&&!op.cancelled.value&&!recovery&&result!=null&&
                (dispatcher as? RemoteDrawingCheckCapability)?.drawingCheckSupported==true&&
                same(op.expected,dispatcher.target.value)&&result.validFor(op.expected))
                    try{adopt(result)}catch(_:Exception){false}else false
            mutable.value=when {
                recovery->RemoteDrawingCheckState(if(closed.value)RemoteDrawingCheckPhase.Closed else RemoteDrawingCheckPhase.Complete,
                    op.expected.identity,"Check the selected Paint canvas; the temporary mark may need manual recovery.",canvasRecoveryNeeded=true)
                closed.value->mutable.value.copy(phase=RemoteDrawingCheckPhase.Closed,cleanupPending=false)
                adopted->RemoteDrawingCheckState(RemoteDrawingCheckPhase.Complete,op.expected.identity,
                    "Mouse drawing check completed. Physical pen support remains Not tested.")
                mutable.value.phase==RemoteDrawingCheckPhase.Complete->mutable.value.copy(cleanupPending=false)
                op.confirmed.value&&!op.cancelled.value->RemoteDrawingCheckState(RemoteDrawingCheckPhase.Complete,
                    op.expected.identity,"No supported drawing observation was admitted.")
                else->RemoteDrawingCheckState()
            }
            current.compareAndSet(op,null)
            busy(false)
            if(op.permitReleased.compareAndSet(false,true))admission.unlock()
            flight.complete(Unit)
        }catch(error:Throwable){
            mutable.value=RemoteDrawingCheckState(if(closed.value)RemoteDrawingCheckPhase.Closed
                else RemoteDrawingCheckPhase.CleanupPending,op.expected.identity,
                "Drawing check cleanup is pending. Keep this session open and retry cleanup.",true)
            flight.completeExceptionally(error)
            throw error
        }finally{op.retirement.compareAndSet(flight,null)}
    }
    suspend fun retryCleanup() {
        val op=current.value?:return
        if(op.task.value?.isCompleted!=true)return
        withContext(NonCancellable){retire(op)}
    }
    suspend fun close():Unit = withContext(NonCancellable) {
        closed.value=true
        this@RemoteDrawingCheck.cancel()
        lifetime.cancelAndJoin()
        current.value?.let{retire(it)}
        mutable.value=mutable.value.copy(phase=RemoteDrawingCheckPhase.Closed,cleanupPending=false)
    }
}
