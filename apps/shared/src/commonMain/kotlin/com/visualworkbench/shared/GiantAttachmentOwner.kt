package com.visualworkbench.shared

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.supervisorScope
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** UI-dispatcher confined attachment owner. Factories consume image ownership on
 * entry, including failed/cancelled construction. Failed view/native retirement
 * stays held here, blocks replacement and must be retained by the parent editor.
 * There is no timeout or failure-to-null conversion that can close the project
 * while native reads or recorded rendering still own its resources. */
public class GiantAttachmentOwner<V : Any>(
    private val attachment: () -> DocumentAttachment?,
    private val create: suspend (GiantImage, SessionResources, WorkflowBinding, Long,
        (WorkflowBinding, Long) -> Boolean) -> V,
    private val closeView: suspend (V) -> Unit,
    private val changed: () -> Unit,
    private val diskCacheBytes: ULong,
    private val ramCacheBytes: ULong,
    private val resources: () -> SessionResources = ::sessionResources,
    private val failedCreationOwner: (Throwable) -> V? = { null },
) {
    private val gate: Mutex = Mutex()
    private var admission: Long = 0L
    private var sealed: Boolean = false
    private var active: Boolean = false
    // Render callbacks read only this published immutable permit, never the
    // UI-confined attachment/project fields. Invalidation revokes it first.
    private val callbackAdmission: MutableStateFlow<Long?> = MutableStateFlow(null)
    private var pending: Job? = null
    private var held: V? = null
    private var image: GiantImage? = null
    private var requested: WorkflowBinding? = null
    private var adopted: DocumentAttachment? = null
    public val current: V? get() {
        if (active) {
            if (adopted?.let { matches(it, binding(it.snapshot)) } == true) return held
            callbackAdmission.value = null
        }
        return null
    }
    public val retaining: Boolean get() = held != null || image != null

    public fun invalidate(): Unit {
        if (admission == Long.MAX_VALUE) sealed = true else admission++
        callbackAdmission.value = null
        active = false; pending?.cancel(); changed()
    }
    /** Called on ordered change BEFORE asynchronous snapshot acquisition. */
    public fun observe(revision: ProjectInfo): Unit {
        val expected = requested ?: return
        if (revision.projectId == expected.projectId &&
            (revision.hostSeq != expected.hostSeq || revision.stateHash != expected.stateHash)) invalidate()
    }
    public suspend fun bind(expected: DocumentAttachment): V? {
        currentCoroutineContext().ensureActive()
        val bound = binding(expected.snapshot)
        if (current != null && adopted?.project === expected.project && adopted?.epoch == expected.epoch && requested == bound) return current
        invalidate(); val ticket = admission; requested = bound
        if (sealed) throw GiantImageFailure(GiantImageFailureKind.Limit)
        return gate.withLock {
            if (!requestCurrent(ticket, expected, bound)) return@withLock null
            retireHeld()
            supervisorScope {
                val work = async(start = CoroutineStart.UNDISPATCHED) { produce(ticket, expected, bound) }
                pending = work
                try { work.await() }
                catch (cancel: CancellationException) {
                    withContext(NonCancellable) { work.join() }
                    currentCoroutineContext().ensureActive()
                    if (!requestCurrent(ticket, expected, bound)) null else throw cancel
                } finally { if (pending === work) pending = null }
            }
        }
    }
    private suspend fun produce(ticket: Long, expected: DocumentAttachment, bound: WorkflowBinding): V? {
        try {
            val capability = expected.project as? WorkbenchGiantImagesProject
                ?: throw GiantImageFailure(GiantImageFailureKind.Unsupported)
            image = capability.openGiantImage(bound, diskCacheBytes, ramCacheBytes)
            currentCoroutineContext().ensureActive()
            if (!requestCurrent(ticket, expected, bound)) { retireHeld(); return null }
            val pool = resources()
            val owned = checkNotNull(image); image = null
            // create consumes owned even when construction throws/cancels.
            callbackAdmission.value = ticket
            held = try {
                create(owned, pool, bound, expected.epoch) { value, epoch ->
                    value == bound && epoch == expected.epoch && callbackAdmission.value == ticket
                }
            } catch (error: Throwable) {
                held = failedCreationOwner(error)
                throw error
            }
            currentCoroutineContext().ensureActive()
            if (!requestCurrent(ticket, expected, bound)) { retireHeld(); return null }
            adopted = expected; active = true; changed(); return held
        } catch (error: Throwable) {
            callbackAdmission.value = null
            active = false
            try { withContext(NonCancellable) { retireHeld() } }
            catch (cleanup: Throwable) { error.addSuppressed(cleanup) }
            throw error
        }
    }
    /** Cancelling this caller cannot detach accepted native or renderer work. */
    public suspend fun detach(): Unit {
        var hideFailure: Throwable? = null
        try { invalidate() } catch (error: Throwable) { hideFailure = error }
        withContext(NonCancellable) { gate.withLock { retireHeld(); requested = null } }
        hideFailure?.let { throw it }
    }
    private suspend fun retireHeld(): Unit {
        held?.let { closeView(it); held = null }
        image?.let { it.close(); image = null }
        adopted = null
    }
    private fun requestCurrent(ticket: Long, expected: DocumentAttachment, bound: WorkflowBinding): Boolean =
        !sealed && ticket == admission && matches(expected, bound)
    private fun matches(expected: DocumentAttachment, bound: WorkflowBinding): Boolean {
        val live = attachment() ?: return false
        val before = expected.snapshot; val now = live.snapshot
        return live.project === expected.project && live.epoch == expected.epoch && binding(now) == bound &&
            now.width == before.width && now.height == before.height && now.bitDepth == before.bitDepth && now.page == before.page
    }
    private fun binding(snapshot: DocumentSnapshot): WorkflowBinding = WorkflowBinding(
        snapshot.render.revision.projectId, snapshot.documentId,
        snapshot.render.revision.hostSeq, snapshot.render.revision.stateHash)
}
