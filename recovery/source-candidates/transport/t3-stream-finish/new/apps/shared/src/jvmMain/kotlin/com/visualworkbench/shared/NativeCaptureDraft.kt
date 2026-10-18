package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import com.visualworkbench.bindings.core.CaptureDraft as NDraft
import com.visualworkbench.bindings.core.CaptureDraftOptions as NOptions
import com.visualworkbench.bindings.core.CaptureDraftInfo as NInfo
import com.visualworkbench.bindings.core.CaptureDraftSave as NPlan
import com.visualworkbench.bindings.core.CaptureDraftSaveInfo as NPlanInfo
import com.visualworkbench.bindings.core.WorkflowException
import kotlinx.coroutines.*

private fun NInfo.common(): CaptureDraftInfo = CaptureDraftInfo(draftId, documentId, streamFrame(frame), previewAssetId, connectionEpoch)
private fun NPlanInfo.common(): CaptureDraftSaveInfo = CaptureDraftSaveInfo(draft.common(), workflowBinding(source), workflowBinding(target),
    originalAssetId, savedAssetId, degraded, transactionId, operationCount)
private suspend fun <T> draftNative(block: suspend () -> T): T = withContext(Dispatchers.Default) {
    try { block() } catch (error: WorkflowException) { throw nativeWorkflowFailure(error) }
    catch (error: IllegalStateException) { throw WorkflowFailure(WorkflowFailureKind.Closed, cause = error) }
}
private suspend fun <T> draftCall(release: suspend (T) -> Unit = {}, block: suspend (Cancellation) -> T): T {
    currentCoroutineContext().ensureActive()
    val token = Cancellation()
    try { return joinedCaptureCall(cancel = token::cancel, release = release) { draftNative { block(token) } } }
    finally { token.destroy() }
}
public actual suspend fun ProjectLink.createCaptureDraft(stream: WorkbenchCaptureStream, documentId: String,
    layerId: String, memoryBudgetBytes: ULong): WorkbenchCaptureDraft {
    currentCoroutineContext().ensureActive()
    val link = this as? NativeFocusAccess ?: throw WorkflowFailure(WorkflowFailureKind.Invalid)
    val source = stream as? NativeCaptureStream ?: throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return source.withNative { receiver ->
        draftCall(release = { it: WorkbenchCaptureDraft -> it.close() }) { token ->
            val handle = link.focusHandle().createCaptureDraft(receiver, NOptions(documentId, layerId, memoryBudgetBytes), token)
            try { NativeDraft(handle) }
            catch (error: Throwable) {
                withContext(NonCancellable) { try { handle.closeDraft() } finally { handle.destroy() } }
                throw error
            }
        }
    }
}
private class NativeDraft(private val handle: NDraft) : WorkbenchCaptureDraft {
    private val owner = CaptureNativeOwner()
    override val info: CaptureDraftInfo = workflowDirect { handle.info().common() }
    override val project: WorkbenchProject = workflowDirect { wrapNativeProject(handle.project()) }
    override suspend fun prepareSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        original: CaptureFrozenBackground, memoryBudgetBytes: ULong): WorkbenchCaptureDraftSave = owner.call {
        val background = original as? NativeCaptureBackground ?: throw WorkflowFailure(WorkflowFailureKind.Invalid)
        val expectedSource = workflowBinding(source); val expectedTarget = workflowBinding(target); val meta = workflowMetadata(metadata)
        background.withNative { pixels ->
            draftCall(release = { it: WorkbenchCaptureDraftSave -> it.close() }) { token ->
                NativeDraftSave(handle.prepareSave(expectedSource, expectedTarget, meta, pixels, memoryBudgetBytes, token))
            }
        }
    }
    override suspend fun prepareDegradedSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        preview: CaptureFrozenBackground, memoryBudgetBytes: ULong): WorkbenchCaptureDraftSave = owner.call {
        val background = preview as? NativeCaptureBackground ?: throw WorkflowFailure(WorkflowFailureKind.Invalid)
        background.withNative { pixels ->
            draftCall(release = { it: WorkbenchCaptureDraftSave -> it.close() }) { token ->
                NativeDraftSave(handle.prepareDegradedSave(workflowBinding(source), workflowBinding(target), workflowMetadata(metadata), pixels, memoryBudgetBytes, token))
            }
        }
    }
    override suspend fun close(): Unit = owner.close({
        var failure: Throwable? = null
        try { draftNative { handle.closeDraft() } } catch (error: Throwable) { failure = error }
        try { project.close() } catch (error: Throwable) {
            val prior = failure
            if (prior == null) failure = error else prior.addSuppressed(error)
        }
        failure?.let { throw it }
    }, handle::destroy)
}
private class NativeDraftSave(private val handle: NPlan) : WorkbenchCaptureDraftSave {
    private val owner = CaptureNativeOwner()
    override val info: CaptureDraftSaveInfo get() = owner.now { workflowDirect { handle.describe().common() } }
    override suspend fun commit(): WorkflowReceipt = owner.call {
        draftCall { token -> handle.commit(token).let { WorkflowReceipt(it.transactionId, workflowInfo(it.revision), it.duplicate) } }
    }
    override suspend fun close(): Unit = owner.close({ draftNative { handle.dispose() } }, handle::destroy)
}