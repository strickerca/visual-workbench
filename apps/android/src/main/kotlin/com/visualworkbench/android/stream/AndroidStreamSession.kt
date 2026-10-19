package com.visualworkbench.android.stream

import android.content.Context
import android.graphics.Bitmap
import com.visualworkbench.android.editor.CanvasScene
import com.visualworkbench.android.input.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.*

/** Root adapter over the existing editor. It borrows ONLY the temporary draft,
 * exposes its ordinary scene/input and admits the native annotation subset.
 * Skip its normal background/giant/Result loaders: this module owns all pixels.
 * Published child scenes have background == null.
 * It never owns/replaces the paired target project or its ProjectLink. */
internal interface AndroidStreamDraftEditor {
    val scenes: StateFlow<CanvasScene?>
    val gestures: CanvasGestureSink
    fun viewport(width: Int, height: Int, density: Float)
    /** Seal new mutations and join the already accepted ordinary edit queue. */
    suspend fun sealEdits()
    /** Retire only child jobs/scenes/input. The native draft owns project.close. */
    suspend fun close()
}

internal data class StreamEditorViewport(val width: Int, val height: Int, val density: Float, val initialCamera: Camera)

internal class AndroidStreamCreationFailure(val owner: AndroidStreamSession) :
    Exception("Stream platform cleanup remains owned")

/** One existing authenticated attachment, with an owned surface and annotation
 * child. Root must close/join this before replacing that link/project. */
internal class AndroidStreamSession private constructor(
    parent: CoroutineScope,
    val annotation: CaptureAnnotationSession,
    private val stillAttached: () -> Boolean,
    admitPenDown: (PenSample, StylusCapabilities) -> Boolean,
    private val openEditor: suspend (WorkbenchProject, CaptureDraftInfo, StreamEditorViewport) -> AndroidStreamDraftEditor,
    resources: SessionResources,
) {
    private val job = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + job + Dispatchers.Main.immediate)
    private val requests = Channel<Unit>(Channel.CONFLATED)
    private val pixels = AndroidStreamPixelFactory(resources)
    private var surfaceCharge: SessionResourceCharge? = null
    private var child: AndroidStreamDraftEditor? = null
    private var childScenes: Job? = null
    private var observer: Job? = null
    private var actor: Job? = null
    private var operation: Job? = null
    private var closing = false
    private var foreground = true
    private val mutableSignal = MutableStateFlow(annotation.stream.signal())
    val signal: StateFlow<CaptureStreamSignal> = mutableSignal.asStateFlow()
    private val mutableIssue = MutableStateFlow<String?>(null)
    val issue: StateFlow<String?> = mutableIssue.asStateFlow()
    private val mutableLossless = MutableStateFlow(false)
    val losslessDisplayed: StateFlow<Boolean> = mutableLossless.asStateFlow()
    private val mutableCloseRequested = MutableStateFlow(false)
    val closeRequested: StateFlow<Boolean> = mutableCloseRequested.asStateFlow()
    fun requestClose() {
        if (annotation.state.value.saving || annotation.state.value.saveRetry) {
            mutableIssue.value = "Settle or retry the same Save before closing this draft."; return
        }
        mutableCloseRequested.value = true
    }
    fun cancelCloseRequest() { mutableCloseRequested.value = false }
    lateinit var canvas: AndroidStreamCanvas
        private set
    private lateinit var lifecycle: StreamCanvasLifecycle<Bitmap>
    private val input = StreamInputFence({ sample, capabilities ->
        foreground && !closing && stillAttached() && ::lifecycle.isInitialized &&
            lifecycle.displayedFrame != null && !lifecycle.sealed && admitPenDown(sample, capabilities)
    }, { lifecycle.stopLiveDraws(); openFrozenEditor() })

    private fun request() { if (!closing) requests.trySend(Unit) }
    private fun start() {
        actor = scope.launch {
            for (ignored in requests) {
                try {
                    if (!foreground || closing || !stillAttached()) continue
                    if (child == null) lifecycle.drawLive()
                    else { try { lifecycle.refreshOriginal() } finally { lifecycle.drawFrozen() } }
                    mutableLossless.value = lifecycle.losslessDisplayed
                } catch (cancel: CancellationException) { throw cancel }
                catch (_: Exception) {
                    mutableIssue.value = if (lifecycle.sealed)
                        "Capture presentation could not be confirmed. Close this stream before reopening; the paired project is retained."
                        else "The capture display could not settle. Retry or close this stream; the paired project is retained."
                    if (lifecycle.sealed) input.block(true)
                }
            }
        }
        observer = scope.launch {
            annotation.stream.signals.collect { value -> mutableSignal.value = value; request() }
        }
        request()
    }

    fun activate() {
        if (closing || !stillAttached()) return
        foreground = true
        try {
            lifecycle.activate()
            val state = annotation.state.value
            input.block(operation?.isActive == true || state.saving || state.saveRetry || state.saved != null || state.editorFailed)
            request()
        } catch (_: Exception) { mutableIssue.value = "This stream attachment has retired. Close it before reconnecting." }
    }
    fun background() {
        foreground = false
        if (::lifecycle.isInitialized) lifecycle.pause()
        input.block(true)
        if (::canvas.isInitialized) canvas.cancelInput()
    }

    /** Retry is for this exact frozen frame; native firstContact is idempotently
     * retained by CaptureAnnotationSession, never replaced by a new source. */
    fun openFrozenEditor() {
        if (closing || operation?.isActive == true || child != null) return
        lifecycle.stopLiveDraws()
        operation = scope.launch {
            var borrowed = false
            try {
                val project = lifecycle.freeze()
                val draft = checkNotNull(annotation.state.value.draft)
                borrowed = true
                val viewport = StreamEditorViewport(canvas.width, canvas.height, canvas.resources.displayMetrics.density,
                    canvas.liveCamera(draft.frame))
                // The returned editor must be registered before cancellation
                // delivery can lose it. Close joins this admitted factory.
                withContext(NonCancellable) { child = openEditor(project, draft, viewport) }
                currentCoroutineContext().ensureActive()
                check(stillAttached() && !closing)
                val editor = checkNotNull(child)
                editor.viewport(canvas.width, canvas.height, canvas.resources.displayMetrics.density)
                canvas.scene = checkNotNull(editor.scenes.value).also {
                    require(it.document.documentId == draft.documentId && it.camera == viewport.initialCamera && it.background == null)
                }
                childScenes = scope.launch(start = CoroutineStart.UNDISPATCHED) {
                    try {
                        editor.scenes.collect { scene ->
                            if (scene != null) require(scene.document.documentId == draft.documentId)
                            canvas.scene = scene; request()
                        }
                    } catch (cancel: CancellationException) { throw cancel }
                    catch (_: Exception) { annotation.editorFailed(); input.block(true)
                        mutableIssue.value = "The draft display changed unexpectedly. Discard this frozen draft or close the stream." }
                }
                input.attach(editor.gestures)
                if (foreground) input.block(false)
                mutableIssue.value = null; request()
            } catch (cancel: CancellationException) {
                if (borrowed) annotation.editorFailed()
                throw cancel
            } catch (_: Exception) {
                if (borrowed) annotation.editorFailed()
                input.block(true)
                if (!borrowed && annotation.state.value.frozen == null && !lifecycle.sealed && foreground && stillAttached()) {
                    // No accepted freeze occurred (for example source changed
                    // before contact). Drop that gesture and wait for a fresh
                    // positively posted frame, rather than stranding live view.
                    try {
                        input.detach(); lifecycle.activate(); request()
                        mutableIssue.value = "A displayed frame is not ready to freeze. Wait for live capture, then touch again."
                    } catch (_: Exception) {
                        input.block(true); mutableIssue.value = "This stream attachment has retired. Close it before reconnecting."
                    }
                } else mutableIssue.value = if (borrowed) "The frozen editor could not open. Discard this draft or close the stream."
                    else "The displayed frame could not freeze. Retry this exact frame or close the stream."
            }
        }
    }

    private suspend fun retireEditor() {
        input.block(true)
        childScenes?.cancelAndJoin(); childScenes = null
        canvas.clear() // Joins the last software post before child paths/project retire.
        child?.let { editor -> editor.close(); if (child === editor) child = null }
        input.detach(); canvas.scene = null; canvas.drawWithFinger(false)
    }

    fun save(confirmedDegraded: Boolean = false) {
        if (closing || operation?.isActive == true || child == null) return
        input.block(true)
        operation = scope.launch {
            try {
                checkNotNull(child).sealEdits()
                lifecycle.save(confirmedDegraded)
                mutableIssue.value = null
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { mutableIssue.value = "Save did not return a durable receipt. Retry the same Save before resuming." }
            finally { request() }
        }
    }
    fun resume() {
        if (closing || operation?.isActive == true) return
        if (annotation.state.value.saveRetry) {
            mutableIssue.value = "Retry the same Save before discarding or resuming this draft."; return
        }
        input.block(true)
        operation = scope.launch {
            try {
                lifecycle.resume(); mutableLossless.value = false; mutableIssue.value = null
                if (foreground) input.block(false) else lifecycle.pause()
                request()
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { mutableIssue.value = "Draft retirement did not finish. Retry discarding or closing; live capture stays stopped." }
        }
    }
    fun retryDisplay() { request() }
    val displayRetryAllowed: Boolean get() = ::lifecycle.isInitialized && !lifecycle.sealed && !closing
    suspend fun requestComputerRegion() { annotation.requestComputerRegion() }

    suspend fun close(): Unit = withContext(NonCancellable + Dispatchers.Main.immediate) {
        closing = true; background()
        observer?.cancelAndJoin(); observer = null
        requests.close(); actor?.cancelAndJoin(); actor = null
        operation?.cancelAndJoin(); operation = null
        if (::lifecycle.isInitialized) lifecycle.close()
        else {
            if (::canvas.isInitialized) canvas.retire()
            pixels.close(); annotation.close()
        }
        surfaceCharge?.let { it.close(); surfaceCharge = null }
        job.cancel()
    }

    companion object {
        /** annotation/stream ownership transfers immediately, including failure.
         * Retain AndroidStreamCreationFailure.owner before reporting cleanup. */
        suspend fun create(context: Context, parent: CoroutineScope, annotation: CaptureAnnotationSession,
            stillAttached: () -> Boolean, admitPenDown: (PenSample, StylusCapabilities) -> Boolean,
            openEditor: suspend (WorkbenchProject, CaptureDraftInfo, StreamEditorViewport) -> AndroidStreamDraftEditor,
            resources: SessionResources = sessionResources()): AndroidStreamSession {
            val owner = AndroidStreamSession(parent, annotation, stillAttached, admitPenDown, openEditor, resources)
            try {
                currentCoroutineContext().ensureActive()
                withContext(NonCancellable + Dispatchers.Main.immediate) {
                    check(stillAttached())
                    owner.surfaceCharge = resources.reserveForeign(STREAM_ANDROID_SURFACE_BYTES)
                    owner.canvas = AndroidStreamCanvas(context, owner.input, { owner.surfaceCharge != null }, owner::request,
                        { width, height, density ->
                            try { owner.child?.viewport(width, height, density) }
                            catch (_: Exception) { annotation.editorFailed(); owner.input.block(true)
                                owner.mutableIssue.value = "The draft viewport could not update. Discard the frozen draft or close the stream." }
                        }, owner::background, owner::activate)
                    owner.lifecycle = StreamCanvasLifecycle(annotation, owner.pixels, owner.canvas::present,
                        owner::retireEditor, owner.canvas::retire, stillAttached)
                }
                currentCoroutineContext().ensureActive()
                parent.coroutineContext[Job]?.ensureActive()
                check(stillAttached())
                owner.start()
                return owner
            } catch (error: Throwable) {
                try { owner.close() } catch (_: Throwable) { throw AndroidStreamCreationFailure(owner) }
                throw error
            }
        }
    }
}
