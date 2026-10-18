package com.visualworkbench.android.stream

import android.app.Application
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal data class AndroidStreamState(val open: Boolean = true, val busy: Boolean = false,
    val stage: CaptureStreamStage = CaptureStreamStage.AwaitingSource, val child: EditorController? = null,
    val failure: String? = null, val closing: Boolean = false)

/** One retained modal owner for one exact parent link/project attachment. All
 * admission lives on Main. close joins real draw, native, and child editor work
 * before the parent is permitted to replace its link or canonical project. */
internal class AndroidStreamController(
    private val application: Application, private val scope: CoroutineScope,
    private val core: WorkbenchCore, val annotation: CaptureAnnotationSession,
) {
    private val mutable = MutableStateFlow(AndroidStreamState())
    val state: StateFlow<AndroidStreamState> = mutable.asStateFlow()
    var inputSealed: Boolean = false
    private var view: StreamCanvasView? = null
    private var child: EditorController? = null
    private var drawWork: Job? = null
    private var action: Job? = null
    private var paused = false
    private var closed = false
    private var cleanupComplete = false
    private val closeGate = Mutex()
    private var originalDisplayed = false
    private val originalGate = Mutex()
    private val observer = scope.launch {
        try { annotation.stream.signals.collect { signal ->
            mutable.value = mutable.value.copy(stage = signal.stage)
            if (annotation.state.value.draft != null && signal.stage == CaptureStreamStage.FrozenLossless)
                refreshOriginal()
        } }
        catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) { message("The original or live connection is unavailable. Keep this draft or discard it explicitly; Save never substitutes preview pixels.") }
    }
    fun message(text: String) { mutable.value = mutable.value.copy(failure = text) }
    fun attachView(value: StreamCanvasView) {
        if (closed) return
        check(view == null || view === value)
        view = value
        child?.let(value::install)
        startDraws()
    }
    fun viewportReady(value: StreamCanvasView) { if (view === value) startDraws() }
    fun detachView(value: StreamCanvasView) { if (view === value) view = null }
    private fun startDraws() {
        if (closed || paused || drawWork != null || view == null || child != null) return
        val job = scope.launch(start = CoroutineStart.LAZY) {
            try {
                while (!closed && !paused && view != null) {
                    val surface = view ?: break
                    val draw = annotation.stream.beginDraw()
                    if (draw == null) { delay(16); continue }
                    var shown = false
                    try { if (!paused && view === surface) surface.show(draw) { shown = true } }
                    finally { withContext(NonCancellable) { try { draw.presented(shown) } finally { draw.close() } } }
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { paused = true; message("Live display stopped at its source or memory boundary. The source was not resized. Close and reconnect after checking the computer.") }
        }
        drawWork = job
        job.invokeOnCompletion { scope.launch(NonCancellable + Dispatchers.Main.immediate) { if (drawWork === job) { drawWork = null; if (!closed && !paused && view != null) startDraws() } } }
        job.start()
    }
    fun freeze(surface: StreamCanvasView) {
        if (closed || action != null || child != null || view !== surface) return
        paused = true // Stop admission synchronously before the first suspension.
        launchAction {
            drawWork?.join() // Never cancel an ambiguous actual presentation.
            val project = annotation.freeze()
            var editor: EditorController? = null
            var retained = false
            try {
                val next = EditorController(application, core, streamDraftOnly = true); editor = next
                next.installStreamDraft(project)
                currentCoroutineContext().ensureActive()
                check(!closed)
                child = next; retained = true
                mutable.value = mutable.value.copy(child = next)
                view?.install(next)
                refreshOriginal()
            } finally { if (!retained) withContext(NonCancellable) { annotation.editorFailed(); editor?.disposeStreamEditor() } }
        }
    }
    fun requestComputerRegion() {
        if (closed || action != null || child != null) return
        launchAction { annotation.requestComputerRegion(); message("Region picker requested. Approve it on the computer and choose the exact region there; no pixels are captured by this request.") }
    }
    fun save(allowDegraded: Boolean = false) {
        val editor = child ?: return
        if (closed || action != null || (!annotation.state.value.lossless && !(allowDegraded && annotation.stream.signal().stage == CaptureStreamStage.FrozenDegraded) && !annotation.state.value.saveRetry) || view?.inputIdle() == false || !editor.streamEditsIdle()) return
        inputSealed = true
        launchAction { editor.settleStreamEdits(); annotation.refreshOriginal(); annotation.save(allowDegraded) }
    }
    fun resume() {
        if (closed || action != null) return
        inputSealed = true
        launchAction {
            retireEditor()
            annotation.resumeAfterEditorDisposed()
            paused = false; originalDisplayed = false; inputSealed = false; startDraws()
        }
    }
    private fun launchAction(body: suspend () -> Unit) {
        if (closed || action != null) return
        mutable.value = mutable.value.copy(busy = true, failure = null)
        val job = scope.launch(start = CoroutineStart.LAZY) {
            try { body() }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { message(annotation.state.value.failure ?: "The operation could not complete. The frozen frame remains owned; retry Save or explicitly discard and resume.") }
        }
        action = job
        job.invokeOnCompletion { scope.launch(NonCancellable + Dispatchers.Main.immediate) { if (action === job) { action = null; mutable.value = mutable.value.copy(busy = false) } } }
        job.start()
    }
    fun cancelAction() { action?.cancel() } // Slot remains owned through joined native completion.
    private suspend fun retireEditor() {
        view?.retireEditor()
        val previous = child
        if (previous != null) {
            previous.disposeStreamEditor()
            if (child === previous) child = null
        }
        mutable.value = mutable.value.copy(child = null)
    }
    private suspend fun refreshOriginal(): Unit = originalGate.withLock {
        annotation.refreshOriginal()
        val editor = child ?: return@withLock
        if (originalDisplayed || !annotation.state.value.lossless || closed) return@withLock
        val pixels = annotation.stream.frozenBackground() ?: return@withLock
        try {
            val identity = annotation.state.value.draft ?: return@withLock
            if (!pixels.lossless || pixels.draftId != identity.draftId || pixels.frame != identity.frame) return@withLock
            val bitmap = readStreamBitmap(pixels)
            var transferred = false
            try {
                if (!closed && child === editor) {
                    // Existing annotations, camera and contact history remain.
                    transferred = true // The child consumes even a refused installation.
                    editor.installStreamOriginal(bitmap, identity.documentId)
                    originalDisplayed = true
                }
            } finally { if (!transferred) bitmap.recycle() }
        } finally { withContext(NonCancellable) { pixels.close() } }
    }
    suspend fun close(): Unit = withContext(NonCancellable) { closeGate.withLock {
        if (cleanupComplete) return@withLock
        closed = true; paused = true; inputSealed = true
        mutable.value = mutable.value.copy(closing = true)
        view?.discardContact()
        // Detachment completes an undrawn offer. A visible already-drawn frame
        // has an immediate receipt; no pending renderer is abandoned.
        view?.retireEditor()
        action?.cancelAndJoin(); observer.cancelAndJoin()
        drawWork?.cancelAndJoin()
        try { retireEditor() } finally { annotation.close() }
        mutable.value = mutable.value.copy(open = false, closing = false)
        cleanupComplete = true
    } }
}
