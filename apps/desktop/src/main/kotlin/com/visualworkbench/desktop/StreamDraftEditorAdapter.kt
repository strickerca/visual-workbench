package com.visualworkbench.desktop

import com.visualworkbench.shared.Camera
import com.visualworkbench.shared.WorkbenchCore
import com.visualworkbench.shared.WorkbenchProject
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CompletableJob
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.nio.file.Path
import kotlin.coroutines.AbstractCoroutineContextElement
import kotlin.coroutines.CoroutineContext

/** A scope marker rejects child-to-parent retirement before it can join itself.
 * Keep ancestor markers when a caller supplies an already owned child scope. */
private class StreamDraftChildContext(val owners: List<Any>) : AbstractCoroutineContextElement(Key) {
    companion object Key : CoroutineContext.Key<StreamDraftChildContext>
}

/** Borrows the native temporary project. The stream parent owns that project;
 * this adapter owns only the child controller and its dedicated coroutine scope. */
internal class StreamDraftEditorAdapter private constructor(
    internal val controller: EditorController,
    private val childJob: CompletableJob,
    private val scopeOwner: Any,
    private val retired: (StreamDraftEditorAdapter) -> Unit,
) {
    private val initialized = CompletableDeferred<Unit>()
    private val initializationJob = CompletableDeferred<Job?>()
    private val retirement = Mutex()
    private val failureLock = Any()
    @Volatile private var initializationFailure: Throwable? = null
    @Volatile private var retiring = false
    @Volatile private var closed = false
    private var registered = false
    private var controllerDisposed = false
    private var scopeRetired = false

    val state: StateFlow<EditorState>
        get() { initializationFailure?.let { throw it }; return controller.state }
    val ready: Boolean
        get() {
            initializationFailure?.let { throw it }
            return initialized.isCompleted && !retiring && !closed && childJob.isActive
        }

    /** Stop new edits and settle admitted mutations before the parent snapshots
     * the borrowed project for Save. Disposal uses the same serialized boundary. */
    suspend fun sealEdits() {
        initialized.await()
        initializationJob.await()?.join()
        retirement.withLock {
            initializationFailure?.let { throw it }
            check(!retiring && !closed) { "The stream draft editor is retiring" }
            controller.settleStreamEdits()
        }
    }

    suspend fun close() {
        check(currentCoroutineContext()[StreamDraftChildContext]?.owners?.none { it === scopeOwner } != false) {
            "The stream parent must retire its editor outside the child scope"
        }
        retiring = true
        withContext(NonCancellable) {
            // Do not cancel initialization: a suspended read can still return a
            // borrowed document after cancellation. Its actual owner settles first.
            initialized.await()
            initializationJob.await()?.join()
            retirement.withLock {
                if (closed) return@withLock
                if (!controllerDisposed) {
                    controller.disposeStreamEditor()
                    controllerDisposed = true
                }
                if (!scopeRetired) {
                    childJob.cancelAndJoin()
                    scopeRetired = true
                }
                // Retain each completed stage across a throwing callback. Retry
                // cannot repeat disposal or advertise retirement before its join.
                if (registered) retired(this@StreamDraftEditorAdapter)
                closed = true
            }
        }
    }

    private fun failInitialization(error: Throwable) = synchronized(failureLock) {
        if (initializationFailure == null) initializationFailure = error
    }

    companion object {
        /** The returned adapter also represents failed initialization. Parent
         * retention precedes every suspended borrow, so failure/currentness checks
         * cannot discard a child that still needs joined retirement. */
        suspend fun create(
            parentScope: CoroutineScope, core: WorkbenchCore, handoff: DesktopHandoff,
            location: LocationPolicy, deviceId: String, project: WorkbenchProject,
            documentId: String, initialCamera: Camera,
            retain: (StreamDraftEditorAdapter) -> Unit,
            retired: (StreamDraftEditorAdapter) -> Unit,
            workRoot: Path = location.path.resolve(".handoff"),
        ): StreamDraftEditorAdapter {
            val childJob = SupervisorJob(parentScope.coroutineContext[Job])
            val scopeOwner = Any()
            val ancestors = parentScope.coroutineContext[StreamDraftChildContext]?.owners.orEmpty()
            val childScope = CoroutineScope(parentScope.coroutineContext + childJob +
                StreamDraftChildContext(ancestors + scopeOwner))
            val adapter = try {
                StreamDraftEditorAdapter(EditorController(childScope, core, handoff, location, deviceId,
                    workRoot, streamDraftOnly = true), childJob, scopeOwner, retired)
            } catch (error: Throwable) {
                // No controller/project borrow escaped construction.
                withContext(NonCancellable) { childJob.cancelAndJoin() }
                throw error
            }
            try {
                retain(adapter)
                adapter.registered = true
            } catch (error: Throwable) {
                // No project acquisition started. Return the failed child rather
                // than discard the scope if the parent's retain callback refused.
                adapter.failInitialization(error)
                adapter.initializationJob.complete(null)
                adapter.initialized.complete(Unit)
                return adapter
            }
            val initialization = childScope.launch {
                try {
                    currentCoroutineContext().ensureActive()
                    adapter.controller.installStreamDraft(project, documentId, initialCamera)
                } catch (error: Throwable) {
                    adapter.failInitialization(error)
                } finally {
                    adapter.initialized.complete(Unit)
                }
            }
            adapter.initializationJob.complete(initialization)
            initialization.invokeOnCompletion { cause ->
                // A cancelled parent can prevent a child body from entering.
                if (!adapter.initialized.isCompleted) {
                    adapter.failInitialization(cause ?: CancellationException("Stream draft initialization did not enter"))
                    adapter.initialized.complete(Unit)
                }
            }
            try { initialization.join() }
            catch (error: CancellationException) {
                // A cancelled factory caller never cancels the independent
                // acquisition or loses its retained, potentially late borrow.
                withContext(NonCancellable) { initialization.join() }
                adapter.failInitialization(error)
            }
            return adapter
        }
    }
}
