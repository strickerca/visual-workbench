package com.visualworkbench.android.stream

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal interface StreamOwnedPixels<T : Any> {
    val value: T
    val frame: CaptureStreamFrame
    suspend fun close()
}

/** Owns failed partial allocations internally until close succeeds. */
internal interface StreamPixelFactory<T : Any> {
    suspend fun copy(source: CaptureStreamPixels): StreamOwnedPixels<T>
    suspend fun close()
}

/** Main-confined; one gate covers draw/post/receipt, physical DOWN freeze and
 * lossless adoption. A Boolean result is known only after platform post returns.
 * An exception during presentation is UNKNOWN, never a fabricated false ACK. */
internal class StreamCanvasLifecycle<T : Any>(
    val annotation: CaptureAnnotationSession,
    private val factory: StreamPixelFactory<T>,
    private val present: suspend (T, CaptureStreamFrame, Boolean) -> Boolean,
    private val retireChild: suspend () -> Unit,
    private val retireSurface: suspend () -> Unit,
    private val stillAttached: () -> Boolean,
) {
    private val stream = annotation.stream
    private val epoch = stream.signal().connectionEpoch
    private val gate = Mutex()
    private val native = linkedSetOf<CaptureStreamPixels>()
    private val images = linkedSetOf<StreamOwnedPixels<T>>()
    private var shown: StreamOwnedPixels<T>? = null
    private var shownLossless = false
    private var live = true
    private var closing = false
    private var closed = false
    private var presentationUnknown = false

    val displayedFrame: CaptureStreamFrame? get() = shown?.frame
    val losslessDisplayed: Boolean get() = shownLossless
    val sealed: Boolean get() = presentationUnknown || closing
    fun pause() { live = false }
    fun activate() {
        check(!sealed && stillAttached() && stream.signal().connectionEpoch == epoch)
        if (annotation.state.value.frozen == null) live = true
    }
    /** Synchronous: accepted physical DOWN seals admission before waiting for a
     * previously entered draw's actual receipt. Hover/navigation never call it. */
    fun stopLiveDraws() { live = false }

    private fun attached() = !closing && stillAttached() && stream.signal().connectionEpoch == epoch
    private suspend fun release(owner: CaptureStreamPixels) { owner.close(); native.remove(owner) }
    private suspend fun release(owner: StreamOwnedPixels<T>) { owner.close(); images.remove(owner) }
    private suspend fun replace(image: StreamOwnedPixels<T>, lossless: Boolean) {
        val previous = shown
        shown = image; shownLossless = lossless
        if (previous != null && previous !== image) release(previous)
    }

    suspend fun drawLive(): Unit = gate.withLock {
        if (!live || sealed || !attached()) return@withLock
        val ticket = stream.beginDraw() ?: return@withLock
        native += ticket
        var candidate: StreamOwnedPixels<T>? = null
        var entered = false
        var receipt = false
        var failure: Throwable? = null
        try {
            candidate = factory.copy(ticket).also { images += it }
            currentCoroutineContext().ensureActive()
            val signal = stream.signal()
            if (!live || !attached() || signal.sourceGeneration != ticket.frame.sourceGeneration || signal.stage != CaptureStreamStage.Live) {
                ticket.presented(false); receipt = true
            } else {
                entered = true
                if (present(checkNotNull(candidate).value, ticket.frame, false)) {
                    // The actual surface already shows these pixels, even if
                    // the following native receipt fails. Retain their owner.
                    val adopted = checkNotNull(candidate); candidate = null
                    replace(adopted, false)
                    ticket.presented(true); receipt = true
                } else { ticket.presented(false); receipt = true }
            }
        } catch (error: Throwable) {
            failure = error
            if (entered && !receipt) { presentationUnknown = true; live = false }
        } finally {
            withContext(NonCancellable) {
                suspend fun settle(body: suspend () -> Unit) {
                    try { body() } catch (error: Throwable) {
                        presentationUnknown = true; live = false
                        val prior = failure; if (prior == null) failure = error else if (prior !== error) prior.addSuppressed(error)
                    }
                }
                // Before entering present there is positive evidence that this
                // ticket never reached the surface. No such guess is made later.
                if (!entered && !receipt) settle { ticket.presented(false) }
                settle { release(ticket) }
                candidate?.let { value -> settle { release(value) } }
            }
        }
        failure?.let { throw it }
    }

    suspend fun freeze(): WorkbenchProject {
        stopLiveDraws()
        return gate.withLock {
            check(attached() && !sealed)
            val project = annotation.freeze()
            val draft = checkNotNull(annotation.state.value.draft)
            require(shown?.frame == draft.frame) { "Frozen draft differs from the actually shown frame" }
            project
        }
    }

    /** Adopt only the complete verified background for this frozen draft. The
     * present adapter copies the latest child scene, preserving provisional
     * geometry, camera and gesture state rather than reopening its document. */
    suspend fun refreshOriginal(): Unit = gate.withLock {
        if (sealed || !attached() || shownLossless) return@withLock
        val draft = annotation.state.value.draft ?: return@withLock
        annotation.refreshOriginal()
        if (!annotation.state.value.lossless) return@withLock
        val source = stream.frozenBackground() ?: return@withLock
        native += source
        var candidate: StreamOwnedPixels<T>? = null
        var entered = false
        try {
            require(source.draftId == draft.draftId && source.frame == draft.frame &&
                source.lossless && source.originalAssetId?.matches(Regex("[0-9a-f]{64}")) == true)
            candidate = factory.copy(source).also { images += it }
            currentCoroutineContext().ensureActive()
            if (!attached() || annotation.state.value.draft != draft) return@withLock
            entered = true
            if (present(checkNotNull(candidate).value, source.frame, true)) {
                val adopted = checkNotNull(candidate); candidate = null
                replace(adopted, true)
            }
        } catch (error: Throwable) {
            if (entered) { presentationUnknown = true; live = false }
            throw error
        } finally {
            withContext(NonCancellable) {
                try { try { release(source) } finally { candidate?.let { release(it) } } }
                catch (error: Throwable) { presentationUnknown = true; live = false; throw error }
            }
        }
    }

    suspend fun drawFrozen(): Unit = gate.withLock {
        if (sealed || annotation.state.value.draft == null) return@withLock
        shown?.let { image ->
            try { present(image.value, image.frame, true) }
            catch (error: Throwable) { presentationUnknown = true; throw error }
        }
    }

    suspend fun save(allowDegraded: Boolean = false): WorkflowReceipt = gate.withLock {
        check(!closing)
        // The Android caller seals/settles its child edits before this call.
        annotation.save(allowDegraded)
    }

    suspend fun resume(): Unit = gate.withLock {
        check(!closing && !presentationUnknown)
        check(!annotation.state.value.saving && !annotation.state.value.saveRetry)
        retireChild() // Includes actual child draw/input retirement, outside root mutation mutex.
        annotation.resumeAfterEditorDisposed()
        val previous = shown; shown = null
        shownLossless = false
        try { previous?.let { release(it) } }
        catch (error: Throwable) { presentationUnknown = true; throw error }
        live = attached()
    }

    /** Root retains this exact lifecycle if any child/native/charge close fails.
     * No parent stream/link teardown follows an unconfirmed surface retirement. */
    suspend fun close() {
        closing = true; live = false
        withContext(NonCancellable) {
            gate.withLock {
                if (closed) return@withLock
                retireChild(); retireSurface()
                for (image in images.toList()) release(image)
                shown = null
                for (owner in native.toList()) release(owner)
                factory.close()
                annotation.close()
                closed = true
            }
        }
    }
}
