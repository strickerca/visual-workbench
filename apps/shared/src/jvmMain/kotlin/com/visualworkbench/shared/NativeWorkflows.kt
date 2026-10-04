package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import com.visualworkbench.bindings.core.CoreException
import com.visualworkbench.bindings.core.WorkflowException
import com.visualworkbench.bindings.core.WorkflowPlan as NPlan
import com.visualworkbench.bindings.core.WorkflowBinding as NBinding
import com.visualworkbench.bindings.core.WorkflowMetadata as NMetadata
import com.visualworkbench.bindings.core.ProjectInfo as NInfo
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import java.util.concurrent.atomic.AtomicBoolean

private val workflowScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
private val workflowSlots = Semaphore(4)
internal fun nativeWorkflowFailure(error: WorkflowException): WorkflowFailure = when(error) {
    is WorkflowException.Memory -> WorkflowFailure(WorkflowFailureKind.Memory,error.estimated,error.budget,error)
    is WorkflowException.Invalid -> WorkflowFailure(WorkflowFailureKind.Invalid,cause=error)
    is WorkflowException.Limit -> WorkflowFailure(WorkflowFailureKind.Limit,cause=error)
    is WorkflowException.Stale -> WorkflowFailure(WorkflowFailureKind.Stale,cause=error)
    is WorkflowException.Missing -> WorkflowFailure(WorkflowFailureKind.Missing,cause=error)
    is WorkflowException.Locked -> WorkflowFailure(WorkflowFailureKind.Locked,cause=error)
    is WorkflowException.Reconciliation -> WorkflowFailure(WorkflowFailureKind.Reconciliation,cause=error)
    is WorkflowException.Detached -> WorkflowFailure(WorkflowFailureKind.Detached,cause=error)
    is WorkflowException.ReusedTransaction -> WorkflowFailure(WorkflowFailureKind.ReusedTransaction,cause=error)
    is WorkflowException.Closed -> WorkflowFailure(WorkflowFailureKind.Closed,cause=error)
    is WorkflowException.Cancelled -> WorkflowFailure(WorkflowFailureKind.Cancelled,cause=error)
    is WorkflowException.Backpressure -> WorkflowFailure(WorkflowFailureKind.Backpressure,cause=error)
    is WorkflowException.Storage -> WorkflowFailure(WorkflowFailureKind.Storage,cause=error)
}
internal fun workflowBinding(value: WorkflowBinding): NBinding {
    if(value.projectId.length!=36 || value.documentId.length!=36 || value.stateHash.length!=64)throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return NBinding(value.projectId,value.documentId,value.hostSeq,value.stateHash)
}
internal fun workflowBinding(value: NBinding): WorkflowBinding = WorkflowBinding(value.projectId,value.documentId,value.hostSeq,value.stateHash)
internal fun workflowMetadata(value: WorkflowMetadata): NMetadata {
    if(value.transactionId.length!=36 || value.deviceId.length>64 || value.firstLamport==0uL || value.createdAtMs<0 || value.createdAtMs>=(1L shl 48))throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return NMetadata(value.transactionId,value.deviceId,value.firstLamport,value.createdAtMs)
}
internal fun workflowInfo(value: NInfo): ProjectInfo = ProjectInfo(value.projectId,value.title,value.deviceId,value.nextLamport,value.canUndo,value.canRedo,value.hostSeq,value.stateHash,value.documentIds)
internal fun <T> workflowDirect(block: () -> T): T = try { block() }
catch(error:WorkflowException){throw nativeWorkflowFailure(error)}
catch(error:CoreException){throw WorkflowFailure(when(error){is CoreException.Closed -> WorkflowFailureKind.Closed;is CoreException.Cancelled -> WorkflowFailureKind.Cancelled;is CoreException.Storage -> WorkflowFailureKind.Storage;is CoreException.Backpressure,is CoreException.Worker -> WorkflowFailureKind.Backpressure;else -> WorkflowFailureKind.Invalid},cause=error)}
catch(error:CancellationException){throw error}
catch(error:IllegalStateException){throw WorkflowFailure(WorkflowFailureKind.Closed,cause=error)}

/** Retain the producer/slot until native work settles, including undelivered
 * opaque handles. No cancelled dispatcher boundary may discard their ownership. */
internal suspend fun <T> settledWorkflow(release: (T) -> Unit = {}, block: suspend (Cancellation) -> T): T =
    ownWorkflowResult(workflowScope, workflowSlots, ::Cancellation, { it.cancel() }, { it.destroy() }, release) { token ->
        try { block(token) }
        catch (error: WorkflowException) { throw nativeWorkflowFailure(error) }
        catch (error: CoreException) { throw WorkflowFailure(when (error) {
            is CoreException.Closed -> WorkflowFailureKind.Closed
            is CoreException.Cancelled -> WorkflowFailureKind.Cancelled
            is CoreException.Storage -> WorkflowFailureKind.Storage
            is CoreException.Backpressure, is CoreException.Worker -> WorkflowFailureKind.Backpressure
            else -> WorkflowFailureKind.Invalid
        }, cause = error) }
        catch (error: CancellationException) { throw error }
        catch (error: IllegalStateException) { throw WorkflowFailure(WorkflowFailureKind.Closed, cause = error) }
    }
internal fun closeWorkflowPlan(value:NPlan){try{value.dispose()}finally{value.destroy()}}
internal fun wrapWorkflowPlan(value:NPlan):WorkbenchWorkflowPlan = try{NativeWorkflowPlan(value)}catch(error:Throwable){closeWorkflowPlan(value);throw error}
private class NativeWorkflowPlan(private val handle:NPlan):WorkbenchWorkflowPlan {
    private val closed=AtomicBoolean(false)
    private fun check(){if(closed.get())throw WorkflowFailure(WorkflowFailureKind.Closed)}
    override fun describe():WorkflowPlanInfo=workflowDirect {check();handle.describe().let{WorkflowPlanInfo(it.transactionId,workflowBinding(it.expected),it.operationCount,it.nextLamport)}}
    override suspend fun commit():WorkflowReceipt {check();return settledWorkflow{cancel->handle.commit(cancel).let{WorkflowReceipt(it.transactionId,workflowInfo(it.revision),it.duplicate)}}}
    override fun close(){if(!closed.getAndSet(true))workflowDirect{closeWorkflowPlan(handle)}}
}
