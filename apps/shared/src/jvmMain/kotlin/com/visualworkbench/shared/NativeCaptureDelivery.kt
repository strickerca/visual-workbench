package com.visualworkbench.shared

import com.visualworkbench.bindings.core.CaptureDeliveryPlan as NativePlan
import com.visualworkbench.bindings.core.prepareCaptureDelivery as nativePrepare
import java.util.concurrent.atomic.AtomicBoolean

private fun releaseCapturePlan(value: NativePlan) { try { value.dispose() } finally { value.destroy() } }
public actual suspend fun prepareCaptureDelivery(source: WorkbenchProject, target: WorkbenchProject,
    sourceBinding: WorkflowBinding, targetBinding: WorkflowBinding, metadata: WorkflowMetadata,
    memoryBudgetBytes: ULong): WorkbenchCaptureDelivery {
    val prepared = settledWorkflow(::releaseCapturePlan) { cancel -> nativePrepare(nativeProjectHandle(source),
        nativeProjectHandle(target), workflowBinding(sourceBinding), workflowBinding(targetBinding),
        workflowMetadata(metadata), memoryBudgetBytes, cancel) }
    return try { NativeDelivery(prepared) } catch (error: Throwable) { releaseCapturePlan(prepared); throw error }
}
private class NativeDelivery(private val handle: NativePlan): WorkbenchCaptureDelivery {
    private val closed = AtomicBoolean(false)
    private fun check() { if (closed.get()) throw WorkflowFailure(WorkflowFailureKind.Closed) }
    override fun describe(): CaptureDeliveryInfo = workflowDirect {
        check(); handle.describe().let { CaptureDeliveryInfo(it.transactionId, workflowBinding(it.source),
            workflowBinding(it.target), it.documentId, it.sourceAssetId, it.captureSessionId, it.frameId,
            it.geometryRevision, it.encodedBytes, it.operationCount) }
    }
    override suspend fun commit(): WorkflowReceipt {
        check()
        return settledWorkflow { token -> handle.commit(token).let { WorkflowReceipt(it.transactionId, workflowInfo(it.revision), it.duplicate) } }
    }
    override fun close() { if (!closed.getAndSet(true)) workflowDirect { releaseCapturePlan(handle) } }
}
public actual suspend fun WorkbenchProject.receivedCapture(afterHostSeq: ULong, memoryBudgetBytes: ULong): ReceivedCapture? =
    settledWorkflow { token -> nativeProjectHandle(this).receivedCapture(afterHostSeq, memoryBudgetBytes, token)?.let {
        ReceivedCapture(workflowBinding(it.binding), it.createdHostSeq, it.sourceAssetId, it.captureSessionId,
            it.frameId, it.geometryRevision, it.originalVerified) } }
