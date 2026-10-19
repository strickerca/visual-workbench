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
/** The outer cancellation token belongs to the caller, while registered child
 * allocation runs independently until actual settlement and adoption. */
private suspend fun <T : OwnedSession> draftAcquire(children: CaptureChildren,
    create: suspend (Cancellation, () -> Unit) -> T): T =
    draftCall(release = { value: T -> value.close() }) { token ->
        children.acquire { retired -> draftNative { create(token, retired) } }
    }

public actual suspend fun ProjectLink.createCaptureDraft(stream: WorkbenchCaptureStream, documentId: String,
    layerId: String, memoryBudgetBytes: ULong): WorkbenchCaptureDraft {
    currentCoroutineContext().ensureActive()
    val link = this as? NativeCaptureAccess ?: throw WorkflowFailure(WorkflowFailureKind.Invalid)
    val source = stream as? NativeCaptureStream ?: throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return source.withNative { receiver ->
        draftAcquire(source.children) { token, retired ->
            NativeDraft(link.focusHandle().createCaptureDraft(receiver, NOptions(documentId, layerId, memoryBudgetBytes), token), retired)
        }
    }
}
private class NativeDraft(private val handle: NDraft, private val retired: () -> Unit) : WorkbenchCaptureDraft, OwnedSession {
    private val children = CaptureChildren()
    private val owner = CaptureNativeOwner()
    override val info: CaptureDraftInfo get() = owner.now { workflowDirect { handle.info().common() } }
    private var borrowedProject: WorkbenchProject? = null
    override val project: WorkbenchProject get() = owner.now { synchronized(this) { borrowedProject ?: workflowDirect { wrapNativeProject(handle.project()) }.also { borrowedProject = it } } }
    override suspend fun prepareSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        original: CaptureFrozenBackground, memoryBudgetBytes: ULong): WorkbenchCaptureDraftSave = owner.call {
        val background = original as? NativeCaptureBackground ?: throw WorkflowFailure(WorkflowFailureKind.Invalid)
        val expectedSource = workflowBinding(source); val expectedTarget = workflowBinding(target); val meta = workflowMetadata(metadata)
        background.withNative { pixels ->
            draftAcquire(children) { token, retired ->
                NativeDraftSave(handle.prepareSave(expectedSource, expectedTarget, meta, pixels, memoryBudgetBytes, token), retired)
            }
        }
    }
    override suspend fun prepareDegradedSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        preview: CaptureFrozenBackground, memoryBudgetBytes: ULong): WorkbenchCaptureDraftSave = owner.call {
        val background = preview as? NativeCaptureBackground ?: throw WorkflowFailure(WorkflowFailureKind.Invalid)
        val expectedSource = workflowBinding(source); val expectedTarget = workflowBinding(target); val meta = workflowMetadata(metadata)
        background.withNative { pixels ->
            draftAcquire(children) { token, retired ->
                NativeDraftSave(handle.prepareDegradedSave(expectedSource, expectedTarget, meta, pixels, memoryBudgetBytes, token), retired)
            }
        }
    }
    override suspend fun close(): Unit { owner.close({
        children.close()
        var failure: Throwable? = null
        try { draftNative { handle.closeDraft() } } catch (error: Throwable) { failure = error }
        try { borrowedProject?.close() } catch (error: Throwable) {
            val prior = failure
            if (prior == null) failure = error else prior.addSuppressed(error)
        }
        failure?.let { throw it }
    }, handle::destroy); retired() }
}
private class NativeDraftSave(private val handle: NPlan, private val retired: () -> Unit) : WorkbenchCaptureDraftSave, OwnedSession {
    private val owner = CaptureNativeOwner()
    override val info: CaptureDraftSaveInfo get() = owner.now { workflowDirect { handle.describe().common() } }
    override suspend fun commit(): WorkflowReceipt = owner.call {
        draftCall { token -> handle.commit(token).let { WorkflowReceipt(it.transactionId, workflowInfo(it.revision), it.duplicate) } }
    }
    override suspend fun close(): Unit { owner.close({ draftNative { handle.dispose() } }, handle::destroy); retired() }
}
/** Normal platform factory: common interaction logic receives the owned native
 * draft and authenticated prompt adapters explicitly, without another link. */
public fun ProjectLink.captureAnnotationSession(target: WorkbenchProject, targetDocumentId: String,
    stream: WorkbenchCaptureStream, newId: () -> String,
    metadata: suspend (ProjectInfo) -> WorkflowMetadata, stillAttached: () -> Boolean): CaptureAnnotationSession =
    CaptureAnnotationSession(target, targetDocumentId, this, stream, newId, metadata, stillAttached,
        { peer, receiver, document, layer -> peer.createCaptureDraft(receiver, document, layer) },
        { request -> requestCaptureRegion(request) })
