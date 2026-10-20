package com.visualworkbench.desktop

import com.visualworkbench.desktop.stream.DesktopStreamContact
import com.visualworkbench.desktop.stream.DesktopStreamView
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlin.coroutines.AbstractCoroutineContextElement
import kotlin.coroutines.CoroutineContext

/** Root wraps the actual renderer immediately after synchronous construction.
 * A failed close must keep this exact port, including a permanently failed leaf. */
internal interface DesktopStreamRenderPort {
    val view: DesktopStreamView? get() = null
    suspend fun stopForContact(viewPointLogical: Point): DesktopStreamContact?
    suspend fun adoptLossless(annotation: CaptureAnnotationSession): Boolean
    suspend fun close()
}

/** Root retains this wrapper from the borrowed adapter's retain callback,
 * before adapter installation suspends. The returned wrapper is that same port. */
internal interface DesktopStreamEditorPort {
    val adapter: StreamDraftEditorAdapter? get() = null
    val ready: Boolean
    suspend fun sealEdits()
    suspend fun close()
}

internal data class DesktopStreamSessionState(
    val signal: CaptureStreamSignal,
    val view: DesktopStreamRenderPort? = null,
    val child: DesktopStreamEditorPort? = null,
    val contact: DesktopStreamContact? = null,
    val busy: Boolean = false,
    val closing: Boolean = false,
    val closed: Boolean = false,
    val cleanupPending: Boolean = false,
    val issue: String? = null,
)

private class DesktopStreamSessionContext(val owners: List<Any>) : AbstractCoroutineContextElement(Key) {
    companion object Key : CoroutineContext.Key<DesktopStreamSessionContext>
}

/** One retained authenticated receiver. Construction is synchronous: the root
 * retains annotation and this owner before start can allocate a platform view.
 * No method takes the parent editor's mutation mutex. Close runs externally,
 * joins admitted work, then retires renderer, borrowed editor and native owner. */
internal class DesktopStreamSession(
    parentScope: CoroutineScope,
    val annotation: CaptureAnnotationSession,
    private val stillAttached: () -> Boolean,
    private val openView: suspend (CoroutineScope, (DesktopStreamRenderPort) -> Unit) -> DesktopStreamRenderPort,
    private val openEditor: suspend (CoroutineScope, WorkbenchProject, CaptureDraftInfo, Camera,
        (DesktopStreamEditorPort) -> Unit) -> DesktopStreamEditorPort,
) {
    private val initial = annotation.stream.signal()
    private val scopeOwner = Any()
    private val scopeJob = SupervisorJob(parentScope.coroutineContext[Job])
    private val scope = CoroutineScope(parentScope.coroutineContext + scopeJob + DesktopStreamSessionContext(
        parentScope.coroutineContext[DesktopStreamSessionContext]?.owners.orEmpty() + scopeOwner))
    private val admission = Any()
    private val gate = Mutex()
    private val retirement = Mutex()
    private val mutable = MutableStateFlow(DesktopStreamSessionState(initial))
    val state: StateFlow<DesktopStreamSessionState> = mutable.asStateFlow()
    @Volatile private var closing = false
    @Volatile private var closed = false
    @Volatile private var view: DesktopStreamRenderPort? = null
    @Volatile private var child: DesktopStreamEditorPort? = null
    private var operation: Job? = null
    private var originalObserver: Job? = null
    private var started = false
    private var annotationRetired = false
    private var resumePending = false
    private var nativeResumed = false

    private fun current(requireSource: Boolean = false) {
        val signal = annotation.stream.signal()
        if (!stillAttached() || signal.connectionEpoch != initial.connectionEpoch ||
            (requireSource && (!signal.sourceAvailable || signal.stage == CaptureStreamStage.SourceClosed ||
                signal.stage == CaptureStreamStage.Closed))) throw WorkflowFailure(WorkflowFailureKind.Stale)
    }

    private suspend fun <T> operate(message: String, allowCleanup: Boolean = false,
        cleanupOnFailure: Boolean = false, body: suspend () -> T): T {
        val work = synchronized(admission) {
            check(!closing && !closed && scopeJob.isActive) { "The capture session is retiring" }
            check(operation?.isCompleted != false) { "A capture session operation is already running" }
            check(allowCleanup || !mutable.value.cleanupPending) { "Capture session cleanup must finish first" }
            mutable.update { it.copy(busy = true, issue = null) }
            scope.async(start = CoroutineStart.LAZY) {
                try {
                    gate.withLock { currentCoroutineContext().ensureActive(); body() }
                } catch (error: Throwable) {
                    mutable.update { it.copy(issue = message,
                        cleanupPending = it.cleanupPending || cleanupOnFailure) }
                    throw error
                }
            }.also { job ->
                operation = job
                job.invokeOnCompletion {
                    synchronized(admission) {
                        if (operation === job) {
                            operation = null
                            mutable.update { it.copy(busy = closing && !closed) }
                        }
                    }
                }
            }
        }
        work.start()
        try { return work.await() }
        catch (error: CancellationException) {
            work.cancel(error)
            // Factory retain callbacks can still register a late resource while
            // their joined cancellation settles. Never discard that ownership.
            withContext(NonCancellable) { work.join() }
            throw error
        }
    }

    private fun retainView(value: DesktopStreamRenderPort) = synchronized(admission) {
        check(view == null || view === value) { "Retire the prior capture renderer before replacement" }
        view = value
        mutable.update { it.copy(view = value) }
    }
    private fun retainChild(value: DesktopStreamEditorPort) = synchronized(admission) {
        check(child == null || child === value) { "Retire the prior borrowed editor before replacement" }
        child = value
        mutable.update { it.copy(child = value) }
    }
    private suspend fun createView() {
        check(view == null)
        val opened = openView(scope, ::retainView)
        retainView(opened) // Also retain a late return before the next cancellation check.
        currentCoroutineContext().ensureActive()
        current()
        started = true
    }

    suspend fun start() = operate("The capture receiver could not open. Close it before retrying.",
        cleanupOnFailure = true) {
        current()
        if (!started) createView()
        startOriginalObserver()
    }

    private fun startOriginalObserver() {
        if (originalObserver != null) return
        val observer = scope.launch(start = CoroutineStart.LAZY) {
            try {
                annotation.stream.signals.collect { signal ->
                    if (closing) return@collect
                    try {
                        gate.withLock {
                            if (closing) return@withLock
                            mutable.update { it.copy(signal = signal) }
                            if (!stillAttached() || signal.connectionEpoch != initial.connectionEpoch) {
                                mutable.update { it.copy(issue = "The paired attachment changed. Retained pixels and any existing Save remain owned until Close.") }
                                return@withLock
                            }
                            if (!signal.sourceAvailable || signal.stage == CaptureStreamStage.SourceClosed ||
                                signal.stage == CaptureStreamStage.Closed) {
                                mutable.update { it.copy(issue = "Capture source closed. Pixels and annotations remain retained; only an existing Save can be retried.") }
                            }
                            val draft = annotation.state.value.draft ?: return@withLock
                            val renderer = view ?: return@withLock
                            if (signal.stage != CaptureStreamStage.FrozenLossless || mutable.value.cleanupPending) return@withLock
                            annotation.refreshOriginal()
                            currentCoroutineContext().ensureActive()
                            current()
                            if (annotation.state.value.draft == draft && annotation.state.value.lossless && view === renderer) {
                                renderer.adoptLossless(annotation)
                                current()
                                require(annotation.state.value.draft == draft)
                            }
                        }
                    } catch (error: CancellationException) { throw error }
                    catch (_: Exception) {
                        mutable.update { it.copy(issue = "The lossless background could not be adopted. The displayed frame remains retained.", cleanupPending = true) }
                    }
                }
            } catch (error: CancellationException) { throw error }
            catch (_: Exception) {
                mutable.update { it.copy(issue = "The capture signal stopped. The displayed frame remains retained until Close.") }
            }
        }
        originalObserver = observer
        observer.start()
    }

    /** Root already admitted this physical contact. Its camera supplies the
     * physical viewport; fit the actual frame only after live draw settlement,
     * since an entered draw may replace the frame sampled before DOWN. */
    suspend fun freeze(viewPointLogical: Point, initialCameraPhysical: Camera): DesktopStreamEditorPort? =
        operate("The displayed frame could not open for annotation. Resume or close this draft.") {
            current(requireSource = true)
            check(started)
            child?.let { existing -> check(existing.ready); return@operate existing }
            check(!annotation.state.value.editorFailed)
            val renderer = checkNotNull(view)
            val contact = renderer.stopForContact(viewPointLogical)
            if (contact == null) {
                // No native freeze occurred. Renew the stopped renderer rather
                // than issue resume for a stream that never acquired a draft.
                try { retireView(); createView() }
                catch (error: Throwable) { mutable.update { it.copy(cleanupPending = true) }; throw error }
                return@operate null
            }
            var borrowed = false
            try {
                val project = annotation.freeze()
                borrowed = true
                val draft = checkNotNull(annotation.state.value.draft)
                require(contact.frame == draft.frame) { "The draft differs from the actual displayed contact frame" }
                mutable.update { it.copy(contact = contact) }
                val actualCamera = streamContactCamera(contact.frame,
                    initialCameraPhysical.viewportWidth, initialCameraPhysical.viewportHeight)
                val opened = openEditor(scope, project, draft, actualCamera, ::retainChild)
                retainChild(opened)
                currentCoroutineContext().ensureActive()
                current(requireSource = true)
                check(opened.ready)
                opened
            } catch (error: Throwable) {
                if (borrowed) annotation.editorFailed()
                if (child != null || error is CancellationException) mutable.update { it.copy(cleanupPending = true) }
                throw error
            }
        }

    suspend fun requestComputerRegion() = operate("The computer region request could not finish.") {
        current()
        check(started)
        annotation.requestComputerRegion()
    }

    suspend fun save(allowDegraded: Boolean = false): WorkflowReceipt = operate(
        "Save did not return a durable receipt. Retry the same Save.",
        allowCleanup = annotation.state.value.saveRetry || annotation.state.value.saved != null,
    ) {
        val saved = annotation.state.value
        if (!saved.saveRetry && saved.saved == null) {
            current(requireSource = true)
            val editor = checkNotNull(child)
            check(editor.ready)
            editor.sealEdits()
            current(requireSource = true)
        }
        annotation.save(allowDegraded)
    }

    private suspend fun retireView() {
        // Stop the transparent annotation draw predicate before removing its
        // background. The child's actual Compose/input jobs join in close().
        child?.adapter?.renderLifetime?.seal()
        val held = view ?: return
        held.close()
        synchronized(admission) {
            if (view === held) { view = null; mutable.update { it.copy(view = null) } }
        }
    }
    private suspend fun retireChild() {
        val held = child ?: return
        held.close()
        synchronized(admission) {
            if (child === held) { child = null; mutable.update { it.copy(child = null) } }
        }
    }

    suspend fun resume() = operate("Draft retirement did not finish. Retry Resume or Close.",
        allowCleanup = true, cleanupOnFailure = true) {
        check(!annotation.state.value.saveRetry) { "Retry the existing Save before resuming" }
        current()
        if (!resumePending) { resumePending = true; nativeResumed = false }
        retireView()
        retireChild()
        if (!nativeResumed) {
            withContext(NonCancellable) {
                if (annotation.state.value.frozen != null || annotation.state.value.draft != null)
                    annotation.resumeAfterEditorDisposed()
                nativeResumed = true
                mutable.update { it.copy(contact = null) }
            }
        }
        currentCoroutineContext().ensureActive()
        current()
        createView()
        resumePending = false; nativeResumed = false
        mutable.update { it.copy(cleanupPending = false, issue = null, signal = annotation.stream.signal()) }
        startOriginalObserver()
    }

    suspend fun close() {
        check(currentCoroutineContext()[DesktopStreamSessionContext]?.owners?.none { it === scopeOwner } != false) {
            "The stream parent must close the session outside its child scope"
        }
        synchronized(admission) {
            closing = true
            mutable.update { it.copy(closing = true, busy = true) }
        }
        withContext(NonCancellable) {
            retirement.withLock {
                if (closed) { mutable.update { it.copy(busy = false) }; return@withLock }
                try {
                    val running = synchronized(admission) { operation }
                    val observer = originalObserver
                    running?.cancel(); observer?.cancel()
                    running?.join(); observer?.join()
                    originalObserver = null
                    gate.withLock {
                        retireView()
                        retireChild()
                        if (!annotationRetired) { annotation.close(); annotationRetired = true }
                    }
                    scopeJob.cancelAndJoin()
                    closed = true
                    mutable.update { it.copy(view = null, child = null, contact = null, busy = false,
                        closed = true, cleanupPending = false, issue = null) }
                } catch (error: Throwable) {
                    mutable.update { it.copy(busy = false, cleanupPending = true,
                        issue = "Capture cleanup is incomplete. Keep the paired project open and retry Close.") }
                    throw error
                }
            }
        }
    }
}
