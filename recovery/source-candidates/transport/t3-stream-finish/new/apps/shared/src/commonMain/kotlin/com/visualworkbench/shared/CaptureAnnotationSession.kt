package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

public data class CaptureAnnotationState(
    public val frozen: CaptureFreezeReceipt? = null,
    public val draft: CaptureDraftInfo? = null,
    public val lossless: Boolean = false,
    public val degradedSaved: Boolean = false,
    public val editorFailed: Boolean = false,
    public val preparing: Boolean = false,
    public val saving: Boolean = false,
    public val saveRetry: Boolean = false,
    public val saved: WorkflowReceipt? = null,
    public val closing: Boolean = false,
    public val failure: String? = null,
)

/** One exact live-project/link attachment. The platform first stops admitting
 * live draws, settles their actual receipts, then calls freeze for an accepted
 * physical contact. Returned project is an ordinary private annotation editor;
 * it never replaces the paired project or owns another link.
 *
 * This owner is UI-dispatcher confined. Its suspend methods serialize complete
 * native settlement; cancellation is joined by the platform-native wrappers.
 * The platform must retire its child editor/render resources before close or
 * resume. Unknown Save outcomes keep the same immutable retry plan. */
public class CaptureAnnotationSession(
    private val target: WorkbenchProject,
    private val targetDocumentId: String,
    private val link: ProjectLink,
    public val stream: WorkbenchCaptureStream,
    private val newId: () -> String,
    private val metadata: suspend (ProjectInfo) -> WorkflowMetadata,
    private val stillAttached: () -> Boolean,
    private val create: suspend (ProjectLink, WorkbenchCaptureStream, String, String) -> WorkbenchCaptureDraft = { peer, receiver, doc, layer ->
        peer.createCaptureDraft(receiver, doc, layer)
    },
) {
    private val gate = Mutex()
    private var disposed = false
    private var cleanupFailed = false
    private var draft: WorkbenchCaptureDraft? = null
    private var original: CaptureFrozenBackground? = null
    private var plan: WorkbenchCaptureDraftSave? = null
    private val epoch = stream.signal().connectionEpoch
    private val mutable = MutableStateFlow(CaptureAnnotationState())
    public val state: StateFlow<CaptureAnnotationState> = mutable.asStateFlow()

    private fun current(): Unit {
        if (disposed || cleanupFailed || state.value.editorFailed) throw WorkflowFailure(WorkflowFailureKind.Closed)
        if (!stillAttached() || stream.signal().connectionEpoch != epoch) throw WorkflowFailure(WorkflowFailureKind.Stale)
    }
    /** Retry after failed preparation reuses the same already-frozen receipt.
     * A timeout permits annotation of that displayed frame; Save stays disabled
     * until that exact original is verified. No preview-as-original fallback. */
    public suspend fun freeze(): WorkbenchProject = gate.withLock {
        currentCoroutineContext().ensureActive(); current()
        draft?.let { return@withLock it.project }
        mutable.value = mutable.value.copy(preparing = true, failure = null)
        try {
            val receipt = mutable.value.frozen ?: stream.firstContact(newId())
                ?: throw WorkflowFailure(WorkflowFailureKind.Missing)
            mutable.value = mutable.value.copy(frozen = receipt)
            current()
            val acquired = create(link, stream, newId(), newId())
            var retained = false
            try {
                currentCoroutineContext().ensureActive(); current()
                require(acquired.info.draftId == receipt.draftId && acquired.info.frame.frameId == receipt.frameId &&
                    acquired.info.frame.captureSessionId == receipt.captureSessionId && acquired.info.frame.sourceGeneration == receipt.sourceGeneration &&
                    acquired.info.connectionEpoch == epoch)
                draft = acquired; retained = true
                mutable.value = mutable.value.copy(draft = acquired.info)
                acquired.project
            } finally { if (!retained) withContext(NonCancellable) { acquired.close() } }
        } catch (cancel: CancellationException) { throw cancel }
        catch (error: Exception) {
            mutable.value = mutable.value.copy(failure = "The frozen annotation draft could not open. Retry this frame or discard it explicitly.")
            throw error
        } finally { mutable.value = mutable.value.copy(preparing = false) }
    }
    public fun estimatedFrameAgeMs(localNowMs:Long):Double? {
        val frame=state.value.draft?.frame ?: return null
        val offset=try { link.status().clockOffsetMs } catch(_:Exception){null}
        return captureFrameAgeMs(frame.capturedAtMs,localNowMs,offset)
    }
    public suspend fun requestComputerRegion():Unit = gate.withLock {
        currentCoroutineContext().ensureActive();current()
        if (draft != null) throw WorkflowFailure(WorkflowFailureKind.Backpressure)
        link.requestCaptureRegion(newId())
    }
    /** Original arrival updates readiness only. It never replaces annotations,
     * changes camera position, or admits a frame from a later source/link. */
    public suspend fun refreshOriginal(): Unit = gate.withLock {
        currentCoroutineContext().ensureActive(); current()
        val active = draft ?: return@withLock
        if (original != null) return@withLock
        val arrived = stream.frozenBackground() ?: return@withLock
        var retained = false
        try {
            currentCoroutineContext().ensureActive(); current()
            if (arrived.draftId != active.info.draftId || arrived.frame != active.info.frame) throw WorkflowFailure(WorkflowFailureKind.Stale)
            if (arrived.lossless && arrived.originalAssetId != null) {
                original = arrived; retained = true
                mutable.value = mutable.value.copy(lossless = true, failure = null)
            }
        } finally { if (!retained) withContext(NonCancellable) { arrived.close() } }
    }
    /** A child that failed after borrowing the draft may have closed that
     * project. Keep the frame sealed for explicit discard; never reopen it. */
    public fun editorFailed() { mutable.value = mutable.value.copy(editorFailed = true,
        failure = "The annotation editor could not open. Discard this frozen draft or close the stream before trying again.") }
    public suspend fun save(allowDegraded: Boolean = false): WorkflowReceipt = gate.withLock {
        currentCoroutineContext().ensureActive()
        if (disposed || cleanupFailed || state.value.editorFailed) throw WorkflowFailure(WorkflowFailureKind.Closed)
        mutable.value.saved?.let { return@withLock it }
        val active = draft ?: throw WorkflowFailure(WorkflowFailureKind.Missing)
        var borrowed: CaptureFrozenBackground? = null
        mutable.value = mutable.value.copy(saving = true, failure = null)
        try {
            val pixels = if (plan != null) original else original ?: if (allowDegraded && stream.signal().stage == CaptureStreamStage.FrozenDegraded) {
                val preview = stream.frozenBackground() ?: throw WorkflowFailure(WorkflowFailureKind.Missing)
                borrowed = preview
                require(!preview.lossless && preview.originalAssetId == null && preview.frame == active.info.frame && preview.draftId == active.info.draftId)
                preview
            } else throw WorkflowFailure(WorkflowFailureKind.Missing)
            // A retained exact native plan can inspect its durable receipt even
            // after a disconnect; new preparation must use the current epoch.
            if (plan == null) {
                current()
                val sourceInfo = active.project.info()
                val targetInfo = target.info()
                val source = WorkflowBinding(sourceInfo.projectId, active.info.documentId, sourceInfo.hostSeq, sourceInfo.stateHash)
                val destination = WorkflowBinding(targetInfo.projectId, targetDocumentId, targetInfo.hostSeq, targetInfo.stateHash)
                val image = checkNotNull(pixels)
                plan = if (image.lossless) active.prepareSave(source, destination, metadata(targetInfo), image)
                    else active.prepareDegradedSave(source, destination, metadata(targetInfo), image)
            }
            currentCoroutineContext().ensureActive()
            mutable.value = mutable.value.copy(saveRetry = true)
            val receipt = checkNotNull(plan).commit()
            // Commit returns only after durable journal publication; no later
            // cancellation check erases the received durable receipt.
            mutable.value = mutable.value.copy(saved = receipt, saveRetry = false, degradedSaved = checkNotNull(plan).info.degraded)
            receipt
        } catch (cancel: CancellationException) { throw cancel }
        catch (error: Exception) {
            mutable.value = mutable.value.copy(failure = "Save did not return a durable receipt. Retry the same Save; the original and annotations remain owned.")
            throw error
        } finally {
            try { withContext(NonCancellable) { borrowed?.close() } }
            finally { mutable.value = mutable.value.copy(saving = false) }
        }
    }
    /** Caller has already retired child renderer/editor. Explicitly discards
     * an unsaved draft or releases a saved one; never closes the paired project. */
    public suspend fun resumeAfterEditorDisposed(): Unit = withContext(NonCancellable) {
        gate.withLock {
            if (disposed) throw WorkflowFailure(WorkflowFailureKind.Closed)
            if (!stillAttached() || stream.signal().connectionEpoch != epoch) throw WorkflowFailure(WorkflowFailureKind.Stale)
            retireDraft()
            stream.resumeAfterDraftDisposed()
            mutable.value = CaptureAnnotationState()
        }
    }
    private suspend fun retireDraft() {
        var failure: Throwable? = null
        suspend fun release(body: suspend () -> Unit) {
            try { body() } catch (error: Throwable) { val prior = failure; if (prior == null) failure = error else prior.addSuppressed(error) }
        }
        // Each failed owner remains retained for a joined retry. Dropping the
        // reference on exception cannot establish actual cleanup.
        release { plan?.let { it.close(); plan = null } }
        release { original?.let { it.close(); original = null } }
        release { draft?.let { it.close(); draft = null } }
        cleanupFailed = failure != null
        failure?.let {
            mutable.value = mutable.value.copy(closing = true, failure = "Draft cleanup is incomplete. Live capture remains sealed; retry closing or discarding this draft.")
            throw it
        }
    }
    /** Seal synchronously before joining entered work. The platform joins this
     * before parent project/link replacement; a late producer cannot resurrect. */
    public suspend fun close(): Unit {
        disposed = true; mutable.value = mutable.value.copy(closing = true)
        withContext(NonCancellable) {
            gate.withLock {
                try { retireDraft() } finally { stream.close() }
            }
        }
    }
}
