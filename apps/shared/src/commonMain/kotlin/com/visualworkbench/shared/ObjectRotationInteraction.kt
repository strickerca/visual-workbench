package com.visualworkbench.shared

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Borrowed from the existing editor. Selection, camera and snapshot are read
 * together on its UI thread; this port owns no document or preview renderer. */
public data class ObjectRotationAttachment(
    public val project: WorkbenchProject,
    public val document: DocumentSnapshot,
    public val epoch: Long,
    public val camera: Camera,
    public val selectedObjectIds: Set<String>,
)

public class ObjectRotationGesture internal constructor(
    public val gestureId: String,
    public val frame: ObjectRotationFrame,
    internal val project: WorkbenchProject,
    internal val selection: Set<String>,
    internal val contact: ObjectRotationContact,
)

/** Submit these absolute commands through the existing CAS edit path using
 * frame.binding and gestureId. The root retains metadata/Lamport ownership. */
public data class ObjectRotationCommit(
    public val project: WorkbenchProject,
    public val frame: ObjectRotationFrame,
    public val gestureId: String,
    public val commands: List<EditCommand>,
)

public enum class ObjectRotationFailure { Stale, Invalid, Cancelled, Unconfirmed }
public data class ObjectRotationState(
    public val active: Boolean = false,
    public val busy: Boolean = false,
    public val preview: Map<String, Transform> = emptyMap(),
    public val failure: ObjectRotationFailure? = null,
)

/** One UI-thread gesture owner. Pointer-up seals synchronously, then makes at
 * most one existing edit callback under the editor's mutation mutex. No drag
 * sample persists and no new undo/retry engine is introduced. A failed callback
 * is unconfirmed: refresh the existing document before starting another edit.
 * Await detach before replacing/closing the borrowed project. */
public class ObjectRotationInteraction(
    parent: CoroutineScope,
    private val mutations: Mutex,
    private val newId: () -> String,
    private val solve: (List<Transform>, Point, Point, Point, Boolean) -> List<Transform>,
    private val current: () -> ObjectRotationAttachment?,
    private val mayMutate: () -> Boolean,
    private val commit: suspend (ObjectRotationCommit) -> Boolean,
) {
    private val scope: CoroutineScope = CoroutineScope(parent.coroutineContext + SupervisorJob(parent.coroutineContext[Job]))
    private val mutableState: MutableStateFlow<ObjectRotationState> = MutableStateFlow(ObjectRotationState())
    public val state: StateFlow<ObjectRotationState> = mutableState.asStateFlow()
    private var gesture: ObjectRotationGesture? = null
    private var operation: Job? = null
    private var detaching: Boolean = false
    private var closed: Boolean = false

    public fun begin(pivot: Point, first: Point): ObjectRotationGesture? {
        if (closed || detaching || !scope.isActive || operation != null || gesture != null ||
            mutableState.value.failure == ObjectRotationFailure.Unconfirmed || !mayMutate()) return null
        val attachment = current() ?: return null
        val selected = attachment.selectedObjectIds.toSet()
        if (selected.isEmpty() || selected.size > 256) return null
        val items = attachment.document.render.items.filter { it.objectId in selected }
        if (items.size != selected.size) return null
        val frame = attachment.document.objectRotationFrame(attachment.epoch, attachment.camera)
        val contact = try { ObjectRotationContact.begin(frame, items, pivot, first, solve) }
        catch (_: Exception) { mutableState.value = ObjectRotationState(failure = ObjectRotationFailure.Invalid); return null }
            ?: return null
        val value = ObjectRotationGesture(newId(), frame, attachment.project, selected, contact)
        // A synchronous root callback may replace the attachment even during ID allocation.
        if (!matches(value) || !scope.isActive || closed || !mayMutate()) { contact.cancel(); return null }
        gesture = value
        mutableState.value = ObjectRotationState(active = true)
        return value
    }

    public fun move(token: ObjectRotationGesture, point: Point, snap15: Boolean): Boolean {
        if (gesture !== token) return false
        if (!scope.isActive || closed || detaching || !matches(token) || !mayMutate()) { retire(ObjectRotationFailure.Stale); return false }
        return try {
            val preview = token.contact.preview(point, token.frame, snap15) ?: return false
            mutableState.value = ObjectRotationState(active = true, preview = preview.toMap())
            true
        } catch (_: Exception) { retire(ObjectRotationFailure.Invalid); false }
    }

    /** Real pointer-up position and current modifier state are authoritative. */
    public fun finish(token: ObjectRotationGesture, point: Point, snap15: Boolean): Boolean {
        if (gesture !== token) return false
        if (!matches(token) || !mayMutate() || !scope.isActive) { retire(ObjectRotationFailure.Stale); return false }
        val commands = try { token.contact.finish(point, token.frame, snap15) }
        catch (_: Exception) { retire(ObjectRotationFailure.Invalid); return false }
            ?: return false
        gesture = null
        if (commands.isEmpty()) { mutableState.value = ObjectRotationState(); return true }
        val request = ObjectRotationCommit(token.project, token.frame, token.gestureId, commands.toList())
        mutableState.value = ObjectRotationState(busy = true, preview = commands.associate {
            val transform = it as EditCommand.SetTransform
            transform.objectId to transform.transform
        })
        var entered = false
        var failure: ObjectRotationFailure? = null
        val launched = scope.launch(start = CoroutineStart.LAZY) {
            try {
                currentCoroutineContext().ensureActive()
                mutations.withLock {
                    if (!matches(token) || !mayMutate()) { failure = ObjectRotationFailure.Stale; return@withLock }
                    currentCoroutineContext().ensureActive()
                    entered = true
                    if (!commit(request)) failure = ObjectRotationFailure.Stale
                }
            } catch (_: CancellationException) {
                failure = if (entered) ObjectRotationFailure.Unconfirmed else ObjectRotationFailure.Cancelled
            } catch (_: Exception) {
                failure = if (entered) ObjectRotationFailure.Unconfirmed else ObjectRotationFailure.Invalid
            }
        }
        // Publish ownership before executing any root callback. Completion also
        // retires a job cancelled by its parent before its body can start.
        operation = launched
        launched.invokeOnCompletion { cause ->
            if (operation === launched) {
                operation = null
                mutableState.value = ObjectRotationState(failure = failure ?: if (cause != null) {
                    if (entered) ObjectRotationFailure.Unconfirmed else ObjectRotationFailure.Cancelled
                } else null)
            }
        }
        launched.start()
        return true
    }

    public fun cancel(): Unit {
        gesture?.contact?.cancel()
        gesture = null
        val pending = operation
        if (pending != null) pending.cancel()
        else if (mutableState.value.failure != ObjectRotationFailure.Unconfirmed) mutableState.value = ObjectRotationState()
    }

    /** Call only after the root admits a fresh native document snapshot after
     * an unconfirmed edit. This never retries or reverses that transaction. */
    public fun reconciled(): Unit {
        if (operation == null && gesture == null && !closed && !detaching) mutableState.value = ObjectRotationState()
    }

    public suspend fun detach(): Unit = withContext(NonCancellable) {
        detaching = true
        try {
            this@ObjectRotationInteraction.cancel()
            operation?.join()
            mutableState.value = ObjectRotationState()
        } finally { detaching = false }
    }

    public suspend fun close(): Unit {
        closed = true
        detach()
        scope.cancel()
    }

    private fun matches(token: ObjectRotationGesture): Boolean {
        val value = current() ?: return false
        return value.project === token.project && value.selectedObjectIds == token.selection &&
            value.document.objectRotationFrame(value.epoch, value.camera) == token.frame
    }
    private fun retire(failure: ObjectRotationFailure): Unit {
        gesture?.contact?.cancel()
        gesture = null
        mutableState.value = ObjectRotationState(failure = failure)
    }
}
