@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.abs

public enum class EraserMode { Stroke, Object }
public enum class EraserChoice { Stroke, Object, Mask }
public enum class EraserInputRoute { Editor, Selection, Eraser, Blocked }
public data class EraserDraft(public val binding: WorkflowBinding, public val camera: Camera,
    public val points: List<Point>, public val radius: Double, public val mode: EraserMode,
    public val width: UInt, public val height: UInt)
public data class EraserUiState(public val active: Boolean = false, public val mode: EraserMode = EraserMode.Stroke,
    public val radius: Double = 12.0, public val ready: Boolean = false, public val busy: Boolean = false,
    public val draft: EraserDraft? = null, public val message: String? = null)
public data class EraserChange(public val revision: ProjectInfo, public val removed: Set<String>,
    public val replacements: Map<String, String?>)
public data class EraserHitQuery(public val document: DocumentSnapshot, public val camera: Camera,
    public val centers: List<Point>, public val radius: Double)

/** Platform path hits are disposable UI work. Only these two guarded native
 * methods mutate a project. Implementations MUST settle native work when the
 * awaiting coroutine is cancelled; the normal shared facades already do so. */
public interface EraserBackend {
    public suspend fun objectHits(query: EraserHitQuery): List<String>
    public suspend fun eraseStrokes(request: VectorEraseEdit): VectorEraseReceipt
    public suspend fun deleteObjects(options: EditOptions, objectIds: List<String>): ProjectInfo
}
public fun projectEraserBackend(project: WorkbenchProject,
    hit: suspend (EraserHitQuery) -> List<String>): EraserBackend = object : EraserBackend {
    private val vector: WorkbenchVectorEraser by lazy { project.vectorEraser() }
    override suspend fun objectHits(query: EraserHitQuery): List<String> = hit(query)
    override suspend fun eraseStrokes(request: VectorEraseEdit): VectorEraseReceipt = vector.erase(request)
    override suspend fun deleteObjects(options: EditOptions, objectIds: List<String>): ProjectInfo =
        project.edit(options, objectIds.map { EditCommand.Delete(it) })
}

/** Loading a selected mask never falls through to object/stroke deletion.
 * Hardware eraser is temporary: it uses the last explicit vector eraser mode,
 * unless the selected-mask tool is active, without changing the pen tool. */
public fun eraserInputRoute(selection: SelectionUiState, eraser: EraserUiState,
    hardwareEraser: Boolean = false): EraserInputRoute {
    if (selection.busy || eraser.busy) return EraserInputRoute.Blocked
    if (selection.active && (!hardwareEraser || selection.tool == SelectionTool.MaskEraser))
        return if (selection.canDraw) EraserInputRoute.Selection else EraserInputRoute.Blocked
    if (hardwareEraser || eraser.active)
        return if (eraser.ready) EraserInputRoute.Eraser else EraserInputRoute.Blocked
    return EraserInputRoute.Editor
}

/** UI-dispatcher confined; no prediction and no per-event work queue. A latest
 * immutable path preview is bounded to 4,096 real points. DOWN owns the camera,
 * visible revision and document until UP. One native transaction is admitted,
 * never automatically retried with fresh IDs. detach settles before project close. */
public class EraserInteraction(
    private val scope: CoroutineScope,
    private val mapPoints: (Camera, Boolean, List<Point>) -> List<Point>,
    private val newId: (ULong) -> String,
    private val nowMs: () -> Long,
    private val afterMutation: suspend (EraserChange?) -> Unit,
) {
    private val mutable: MutableStateFlow<EraserUiState> = MutableStateFlow(EraserUiState())
    public val state: StateFlow<EraserUiState> = mutable.asStateFlow()
    private var backend: EraserBackend? = null
    private var document: DocumentSnapshot? = null
    private var projectId: String? = null
    private var observed: ProjectInfo? = null
    private var attachment: Long = 0
    private var sequence: ULong = 0u
    private var admission: Long = 0
    private var camera: Camera? = null
    private var owner: Job? = null
    private var operation: Job? = null
    private var detaching: Boolean = false
    private var gesture: Gesture? = null
    private data class Gesture(val backend: EraserBackend, val document: DocumentSnapshot, val camera: Camera,
        val attachment: Long, val admission: Long, val mode: EraserMode, val radius: Double,
        val points: MutableList<Point>)

    public fun bind(value: EraserBackend, snapshot: DocumentSnapshot, epoch: Long, eventSequence: ULong) {
        if (detaching) return
        check(backend == null || backend === value) { "Detach eraser work before replacing the project" }
        if (backend === value && attachment == epoch && eventSequence < sequence) return
        if (backend === value && attachment == epoch && eventSequence == sequence &&
            observed?.let { !sameEraseRevision(it, snapshot.render.revision) } == true) {
            invalidate(); document = null; mutable.value = state.value.copy(ready = false); return
        }
        if (backend === value && attachment == epoch && eventSequence == sequence && document?.let {
            it.documentId == snapshot.documentId && sameEraseRevision(it.render.revision, snapshot.render.revision)
        } == true) return
        if (backend == null) owner = SupervisorJob(scope.coroutineContext[Job])
        if (backend !== value || attachment != epoch || eventSequence > sequence) observed = null
        backend = value; attachment = epoch; sequence = eventSequence; projectId = snapshot.render.revision.projectId
        invalidate(); document = snapshot
        mutable.value = state.value.copy(ready = true)
    }
    public fun observe(info: ProjectInfo, eventSequence: ULong) {
        val current = document
        if (info.projectId != projectId || eventSequence <= sequence) return
        sequence = eventSequence; observed = info
        if (current == null || !sameEraseRevision(info, current.render.revision)) {
            invalidate(); document = null; mutable.value = state.value.copy(ready = false)
        }
    }
    public fun viewport(value: Camera) {
        if (value != camera) { camera = value; invalidate() }
    }
    private fun invalidate() { admission++; cancelGesture() }
    public fun active(value: Boolean) { cancelGesture(); mutable.value = state.value.copy(active = value) }
    public fun tool(value: EraserMode) {
        cancelGesture(); mutable.value = state.value.copy(active = true, mode = value, message = null)
    }
    public fun radius(value: Double) {
        require(value.isFinite() && value in .5..256.0)
        cancelGesture(); mutable.value = state.value.copy(radius = value)
    }
    public fun dismissMessage() { mutable.value = state.value.copy(message = null) }

    public fun begin(screenPoint: Point, capturedCamera: Camera, hardwareEraser: Boolean = false): Boolean {
        if ((!state.value.active && !hardwareEraser) || !state.value.ready || state.value.busy ||
            !scope.isActive || owner?.isActive != true || capturedCamera != camera || !validEraseCamera(capturedCamera)) return false
        val source = document ?: return false; val target = backend ?: return false
        if (source.render.items.size > 4_096) { fail(WorkflowFailure(WorkflowFailureKind.Limit)); return false }
        val point = mapped(screenPoint, capturedCamera) ?: return false
        gesture = Gesture(target, source, capturedCamera, attachment, admission, state.value.mode, state.value.radius, mutableListOf(point))
        preview(); return true
    }
    public fun append(screenPoint: Point) {
        val run = gesture ?: return
        if (!current(run)) { cancelGesture(); return }
        val point = mapped(screenPoint, run.camera) ?: run { cancelGesture(); return }
        if (point != run.points.last()) {
            if (run.points.size == 4_096) { cancelGesture(); fail(WorkflowFailure(WorkflowFailureKind.Limit)); return }
            run.points += point
        }
        preview()
    }
    private fun mapped(point: Point, view: Camera): Point? {
        if (!validErasePoint(point)) return null
        return runCatching { mapPoints(view, true, listOf(point)).singleOrNull() }.getOrNull()?.takeIf(::validErasePoint)
    }
    private fun preview() {
        val run = gesture ?: return
        mutable.value = state.value.copy(draft = EraserDraft(run.document.eraseBinding(), run.camera,
            run.points.toList(), run.radius, run.mode, run.document.width, run.document.height))
    }
    public fun cancelGesture() { gesture = null; mutable.value = state.value.copy(draft = null) }

    public fun finish(screenPoint: Point? = null): Boolean {
        if (screenPoint != null) append(screenPoint)
        val run = gesture ?: return false
        cancelGesture()
        if (!current(run) || operation?.isActive == true) return false
        val lifetime = owner?.takeIf { it.isActive } ?: return false
        val points = run.points.toList()
        val now = nowMs(); if (now < 0) return false
        val info = run.document.render.revision
        val metadata = WorkflowMetadata(newId(now.toULong()), info.deviceId, info.nextLamport, now)
        mutable.value = state.value.copy(busy = true, message = "Checking the erase…")
        operation = CoroutineScope(scope.coroutineContext + lifetime).launch(start = CoroutineStart.UNDISPATCHED) {
            var receipt: EraserChange? = null
            var failure: Exception? = null
            try {
                currentCoroutineContext().ensureActive()
                if (!current(run)) throw WorkflowFailure(WorkflowFailureKind.Stale)
                when (run.mode) {
                    EraserMode.Stroke -> {
                        val targets = strokeTargets(run.document, points, run.radius)
                        if (targets.isNotEmpty()) {
                            // The exact IDs, samples, revision and metadata are captured once.
                            val request = VectorEraseEdit(run.document.eraseBinding(), metadata,
                                targets.map { VectorEraseTarget(it, newId(now.toULong())) }, points, run.radius)
                            currentCoroutineContext().ensureActive()
                            if (!current(run)) throw WorkflowFailure(WorkflowFailureKind.Stale)
                            val result = run.backend.eraseStrokes(request)
                            if (result.transactionId != metadata.transactionId || result.revision.projectId != info.projectId ||
                                result.changed.size > targets.size || result.changed.map { it.originalId }.distinct().size != result.changed.size ||
                                result.changed.any { changed -> request.targets.none { it.objectId == changed.originalId &&
                                    (changed.outlineId == null || it.replacementId == changed.outlineId) } })
                                throw WorkflowFailure(WorkflowFailureKind.Storage)
                            receipt = EraserChange(result.revision, result.changed.mapTo(mutableSetOf()) { it.originalId },
                                result.changed.associate { it.originalId to it.outlineId })
                        }
                    }
                    EraserMode.Object -> {
                        val query = EraserHitQuery(run.document, run.camera, points, run.radius)
                        admitEraserHit(query)
                        val found = run.backend.objectHits(query)
                        if (found.size > 512) throw WorkflowFailure(WorkflowFailureKind.Limit)
                        val ids = found.toList()
                        if (ids.distinct().size != ids.size || ids.any { id ->
                            run.document.render.items.none { it.objectId == id && !it.locked }
                        }) throw WorkflowFailure(WorkflowFailureKind.Limit)
                        currentCoroutineContext().ensureActive()
                        if (!current(run)) throw WorkflowFailure(WorkflowFailureKind.Stale)
                        if (ids.isNotEmpty()) {
                            val options = EditOptions(metadata.transactionId, run.document.documentId, metadata.deviceId,
                                metadata.firstLamport, now, info.hostSeq, info.stateHash)
                            val result = run.backend.deleteObjects(options, ids)
                            if (result.projectId != info.projectId) throw WorkflowFailure(WorkflowFailureKind.Storage)
                            receipt = EraserChange(result, ids.toSet(), emptyMap())
                        }
                    }
                }
            } catch (cancel: CancellationException) { failure = cancel; throw cancel }
            catch (error: Exception) { failure = error }
            finally {
                // Accepted receipt bookkeeping and authoritative refresh are
                // independent of caller cancellation. No second edit is issued.
                withContext(NonCancellable) {
                    if (!detaching && backend === run.backend && attachment == run.attachment) {
                        try { afterMutation(receipt) }
                        catch (error: Exception) { failure = error; mutable.value = state.value.copy(ready = false) }
                        finally {
                            mutable.value = state.value.copy(busy = false)
                            if (failure != null) fail(checkNotNull(failure))
                            else mutable.value = state.value.copy(message = if (receipt?.removed?.isNotEmpty() == true)
                                if (run.mode == EraserMode.Stroke) "Erase saved as editable vector outlines. Undo restores the original strokes."
                                else "Objects erased. Undo restores them." else "No editable marks were touched.")
                        }
                    }
                }
            }
        }
        return true
    }
    private fun current(run: Gesture): Boolean = !detaching && backend === run.backend && attachment == run.attachment &&
        admission == run.admission && document?.let { it.documentId == run.document.documentId &&
            sameEraseRevision(it.render.revision, run.document.render.revision) } == true
    public fun cancelOperation() {
        cancelGesture(); operation?.cancel()
        mutable.value = state.value.copy(message = "Cancelling; checking whether the erase was already saved…")
    }
    public suspend fun detach(): Unit = withContext(NonCancellable) {
        detaching = true; invalidate(); owner?.cancel(); operation?.cancelAndJoin()
        operation = null; owner = null; backend = null; document = null; camera = null; projectId = null; observed = null
        mutable.value = EraserUiState(mode = state.value.mode, radius = state.value.radius)
        detaching = false
    }
    private fun fail(error: Exception) {
        val kind = (error as? WorkflowFailure)?.kind
        val text = when {
            error is CancellationException || kind == WorkflowFailureKind.Cancelled -> "Erase cancelled. The refreshed document shows any operation already saved."
            kind == WorkflowFailureKind.Stale || error is CoreFailure && error.kind == CoreFailureKind.Invalid -> "The document changed. Review it and draw a new erase gesture."
            kind == WorkflowFailureKind.Memory || kind == WorkflowFailureKind.Limit -> "This erase exceeds the geometry or work budget. Use a shorter gesture over fewer objects; no resizing was applied."
            kind == WorkflowFailureKind.Locked -> "A target or layer is locked. Refresh before erasing."
            kind == WorkflowFailureKind.Invalid -> "Stroke erasing supports raw strokes and canonical fill-only vector outlines. Use Object mode for other shapes."
            else -> "The erase could not finish. The document was refreshed; inspect it before trying another gesture."
        }
        mutable.value = state.value.copy(message = text)
    }
}

private fun DocumentSnapshot.eraseBinding(): WorkflowBinding = WorkflowBinding(render.revision.projectId, documentId,
    render.revision.hostSeq, render.revision.stateHash)
private fun sameEraseRevision(a: ProjectInfo, b: ProjectInfo): Boolean = a.projectId == b.projectId &&
    a.hostSeq == b.hostSeq && a.stateHash == b.stateHash && a.nextLamport == b.nextLamport
private fun validErasePoint(point: Point): Boolean = point.x.isFinite() && point.y.isFinite() && abs(point.x) <= 1e9 && abs(point.y) <= 1e9
private fun validEraseCamera(camera: Camera): Boolean = validErasePoint(camera.center) && camera.scale.isFinite() && camera.scale > 0 &&
    camera.rotation.isFinite() && camera.viewportWidth.isFinite() && camera.viewportHeight.isFinite() && camera.viewportWidth > 0 && camera.viewportHeight > 0
private fun strokeTargets(document: DocumentSnapshot, points: List<Point>, radius: Double): List<String> {
    val left = points.minOf { it.x } - radius; val right = points.maxOf { it.x } + radius
    val top = points.minOf { it.y } - radius; val bottom = points.maxOf { it.y } + radius
    val result = mutableListOf<String>()
    for (item in document.render.items) {
        val eligible = item.shape is Shape.Stroke || item.shape is Shape.Polygon && item.shape.closed &&
            item.style.width == 0.0 && item.style.fill != null && !item.style.screenConstantWidth
        if (item.locked || !eligible || item.bounds.x > right || item.bounds.x + item.bounds.width < left ||
            item.bounds.y > bottom || item.bounds.y + item.bounds.height < top) continue
        if (result.size == 32) throw WorkflowFailure(WorkflowFailureKind.Limit)
        result += item.objectId
    }
    return result
}

/** Reject before allocating platform paths. Limits include item headers,
 * outline commands, contour headers, and the complete real sample sweep.
 * A unit charges a cubic as four points; at most 512 objects/real samples,
 * 16,384 path units and 1M pair-work units are admitted. The platform path
 * engine owns its scratch; these are complexity limits, not an allocator cap.
 * This is UI hit testing, not replacement geometry. */
public fun admitEraserHit(query: EraserHitQuery) {
    if (query.document.render.items.size > 512 || query.centers.size !in 1..512 ||
        !query.radius.isFinite() || query.radius !in .5..256.0 || !validEraseCamera(query.camera) ||
        query.centers.any { !validErasePoint(it) }) throw WorkflowFailure(WorkflowFailureKind.Limit)
    var units = 0L
    for (item in query.document.render.items) {
        val count = when (val shape = item.shape) {
            is Shape.Stroke -> item.contours.x.size.toLong() + item.contours.y.size + item.contours.ends.size
            is Shape.Text -> shape.outline.size.toLong() * 4 + shape.text.length
            is Shape.Line -> shape.points.size.toLong()
            is Shape.Arrow -> shape.points.size.toLong() + 8
            is Shape.Polygon -> shape.points.size.toLong()
            is Shape.Marker -> 256L
            else -> 32L
        }
        units += 16 + count
        if (units > 16_384L || units * query.centers.size.toLong() > 1_000_000L)
            throw WorkflowFailure(WorkflowFailureKind.Limit)
    }
}

/** Shared mode arbitration for both editors. SelectionController continues to
 * own mask reads, writes, PNG export and picker tickets. Activating a selection
 * tool retires vector eraser activation; returning to drawing cannot revive it. */
public class EraserToolbox(public val interaction: EraserInteraction,
    public val selection: SelectionInteraction, scope: CoroutineScope) {
    init { scope.launch {
        var wasActive = false
        selection.state.collect { if (it.active && !wasActive) interaction.active(false); wasActive = it.active }
    } }
    public fun choose(value: EraserChoice, selectedObject: String? = null) {
        if (interaction.state.value.busy || selection.state.value.busy) return
        interaction.cancelGesture(); selection.cancelGesture()
        when (value) {
            EraserChoice.Stroke, EraserChoice.Object -> {
                selection.active(false)
                interaction.tool(if (value == EraserChoice.Stroke) EraserMode.Stroke else EraserMode.Object)
            }
            EraserChoice.Mask -> {
                interaction.active(false)
                if (selectedObject != null && selection.state.value.selection?.selection?.objectId != selectedObject)
                    selection.useObject(selectedObject)
                selection.tool(SelectionTool.MaskEraser)
                selection.brush(interaction.state.value.radius, 255u)
            }
        }
    }
    public fun drawing() { interaction.active(false); selection.active(false) }
    public fun radius(value: Double) {
        interaction.radius(value)
        if (selection.state.value.active && selection.state.value.tool == SelectionTool.MaskEraser)
            selection.brush(value, selection.state.value.opacity)
    }
    public fun route(hardwareEraser: Boolean = false): EraserInputRoute =
        eraserInputRoute(selection.state.value, interaction.state.value, hardwareEraser)
    public fun cancel() { interaction.cancelOperation(); selection.cancelOperation() }
}
