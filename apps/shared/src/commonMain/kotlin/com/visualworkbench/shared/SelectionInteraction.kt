package com.visualworkbench.shared

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

public enum class SelectionTool { Rectangle, Lasso, Paint, MaskEraser }
public enum class SelectionRefinement { Invert, Expand, Shrink, Feather }
public data class SelectionDraft(public val tool: SelectionTool, public val points: List<Point>, public val radius: Double)
public data class SelectionPreview(public val binding: SelectionBinding, public val selection: SelectionVersion,
    public val tiles: List<SelectionTile>, public val limitedToViewport: Boolean)
public data class SelectionUiState(
    public val active: Boolean = false,
    public val tool: SelectionTool = SelectionTool.Rectangle,
    public val combine: SelectionCombine = SelectionCombine.Add,
    public val radius: Double = 12.0,
    public val opacity: UByte = 255u,
    public val refinementRadius: UInt = 4u,
    public val document: SelectionDocument? = null,
    public val selection: SelectionSnapshot? = null,
    public val preview: SelectionPreview? = null,
    public val draft: SelectionDraft? = null,
    public val busy: Boolean = false,
    public val loading: Boolean = false,
    public val message: String? = null,
) {
    public val canDraw: Boolean get() = active && document != null && !busy && !loading &&
        (selection?.editable != false) && (tool != SelectionTool.MaskEraser || selection != null)
    public val canRefine: Boolean get() = document != null && selection?.editable == true && !busy && !loading
}

/** An immutable picker request; selecting a destination never changes its source. */
public data class SelectionSaveTicket(public val attachment: Long, public val admission: Long,
    public val binding: SelectionBinding, public val selection: SelectionVersion,
    public val kind: SelectionExportKind, public val region: SelectionRegion?)

/** Main/UI-dispatcher-confined controller. bind/observe are called by the editor's
 * existing ordered refresh path. detach MUST settle before WorkbenchProject.close.
 * Read requests are conflated; one mutation/export and one reader are admitted.
 * No platform classes, generated bindings, prediction, or duplicate mask math. */
public class SelectionInteraction(
    private val core: WorkbenchCore,
    private val scope: CoroutineScope,
    private val nowMs: () -> Long,
    private val afterMutation: suspend (SelectionReceipt?) -> Unit,
    private val capability: (WorkbenchProject) -> WorkbenchSelections = { it.selections() },
) {
    private val mutableState: MutableStateFlow<SelectionUiState> = MutableStateFlow(SelectionUiState())
    public val state: StateFlow<SelectionUiState> = mutableState.asStateFlow()
    private var project: WorkbenchProject? = null
    private var api: WorkbenchSelections? = null
    private var attachment: Long = 0
    private var admission: Long = 0
    private var previewAdmission: Long = 0
    private var sequence: ULong = 0u
    private var displayed: DocumentSnapshot? = null
    private var documentId: String? = null
    private var selectedId: String? = null
    private var camera: Camera? = null
    private var gesture: Gesture? = null
    private var owner: Job? = null
    private var reader: Job? = null
    private var operation: Job? = null
    private var wakes: Channel<Unit>? = null
    private var detaching: Boolean = false
    private val calls: Mutex = Mutex()
    private data class Gesture(val admission: Long, val camera: Camera, val document: SelectionDocument,
        val target: SelectionTarget, val tool: SelectionTool, val combine: SelectionCombine,
        val radius: Double, val opacity: UByte, val points: MutableList<Point>)

    /** Same project only until detach returns. A new visible hash is valid even
     * when hostSeq did not change (optimistic pending client transactions). */
    public fun bind(value: WorkbenchProject, document: DocumentSnapshot, epoch: Long, eventSequence: ULong) {
        if (detaching) return
        check(project == null || project === value) { "Detach selection work before replacing the project" }
        if (project === value && attachment == epoch && eventSequence < sequence) return
        if (project === value && attachment == epoch && displayed?.documentId == document.documentId &&
            displayed?.render?.revision?.let { sameRevision(it, document.render.revision) } == true &&
            eventSequence == sequence && (!state.value.active || mutableState.value.document != null)) return
        val sameDocument = project === value && attachment == epoch && documentId == document.documentId
        if (project == null) {
            project = value // Native capability is acquired only after explicit tool activation.
            val job = SupervisorJob(scope.coroutineContext[Job]); owner = job
            val channel = Channel<Unit>(Channel.CONFLATED); wakes = channel
            reader = CoroutineScope(scope.coroutineContext + job).launch {
                for (ignored in channel) {
                    val requestAdmission = admission
                    val requestObject = selectedId
                    try { reload() }
                    catch (error: CancellationException) { throw error }
                    catch (error: Exception) { if (requestAdmission == admission && requestObject == selectedId) setFailure(error) }
                }
            }
        }
        if (!sameDocument) selectedId = null
        attachment = epoch; sequence = eventSequence; displayed = document; documentId = document.documentId
        invalidate(); mutableState.value = mutableState.value.copy(loading = state.value.active)
        wakes?.trySend(Unit)
    }

    /** Called immediately on an ordered document change, BEFORE an asynchronous
     * document refresh. A stale overlay/export is removed during that gap. */
    public fun observe(revision: ProjectInfo, eventSequence: ULong) {
        val current = displayed ?: return
        if (revision.projectId != current.render.revision.projectId || eventSequence <= sequence) return
        sequence = eventSequence
        if (!sameRevision(revision, current.render.revision)) {
            invalidate(); displayed = null
            mutableState.value = mutableState.value.copy(loading = true)
        }
    }

    private fun invalidate() {
        admission++; previewAdmission++; gesture = null
        mutableState.value = mutableState.value.copy(document = null, selection = null, preview = null, draft = null)
    }

    public fun active(value: Boolean) {
        cancelGesture(); admission++; previewAdmission++
        mutableState.value = mutableState.value.copy(active = value, preview = null,
            document = if (value) state.value.document else null,
            selection = if (value) state.value.selection else null,
            loading = value && (state.value.loading || state.value.document == null || selectedSnapshotPending()))
        wakes?.trySend(Unit)
    }
    public fun tool(value: SelectionTool) {
        cancelGesture(); mutableState.value = mutableState.value.copy(active = true, tool = value, message = null,
            loading = state.value.loading || state.value.document == null || selectedSnapshotPending())
        wakes?.trySend(Unit)
    }
    private fun selectedSnapshotPending(): Boolean = selectedId != null && state.value.selection?.selection?.objectId != selectedId
    public fun combine(value: SelectionCombine) {
        cancelGesture(); mutableState.value = mutableState.value.copy(combine = value)
    }
    public fun brush(radius: Double, opacity: UByte = state.value.opacity) {
        require(radius.isFinite() && radius in 0.5..256.0)
        cancelGesture(); mutableState.value = mutableState.value.copy(radius = radius, opacity = opacity)
    }
    public fun refinementRadius(value: UInt) {
        require(value in 1u..256u); mutableState.value = mutableState.value.copy(refinementRadius = value)
    }
    public fun dismissMessage() { mutableState.value = mutableState.value.copy(message = null) }
    public fun newSelection() {
        if (state.value.busy) return
        selectedId = null; cancelGesture(); previewAdmission++
        mutableState.value = mutableState.value.copy(selection = null, preview = null, combine = SelectionCombine.Add,
            tool = if (state.value.tool == SelectionTool.MaskEraser) SelectionTool.Paint else state.value.tool, message = null)
        wakes?.trySend(Unit)
    }
    /** Existing Guide items include crop/vector guides too; native snapshot
     * validation is authoritative and refuses anything other than an editable mask. */
    public fun useObject(objectId: String) {
        if (state.value.busy || objectId.length > 64) return
        selectedId = objectId; cancelGesture(); previewAdmission++
        mutableState.value = mutableState.value.copy(active = true, selection = null, preview = null, loading = true)
        wakes?.trySend(Unit)
    }
    public fun viewport(value: Camera) {
        if (camera == value) return
        cancelGesture()
        camera = value; previewAdmission++
        mutableState.value = mutableState.value.copy(preview = null)
        wakes?.trySend(Unit)
    }

    public fun begin(screenPoint: Point, capturedCamera: Camera): Boolean {
        if (!state.value.canDraw || selectedSnapshotPending() || owner?.isActive != true) return false
        val document = state.value.document ?: return false
        if (!validCamera(capturedCamera) || !finite(screenPoint)) return false
        val point = runCatching { core.mapPoints(capturedCamera, true, listOf(screenPoint)).singleOrNull() }.getOrNull() ?: return false
        if (!finite(point)) return false
        val current = state.value
        if (current.selection == null && current.combine != SelectionCombine.Add) {
            mutableState.value = current.copy(message = "Create a selection with Add before subtracting or intersecting."); return false
        }
        val time = nowMs(); if (time < 0) return false
        val target = current.selection?.let { SelectionTarget.Existing(it.selection) } ?:
            SelectionTarget.New(core.newId(time.toULong()), core.newId(time.toULong()))
        gesture = Gesture(admission, capturedCamera, document, target, current.tool,
            if (current.tool == SelectionTool.MaskEraser) SelectionCombine.Subtract else current.combine,
            current.radius, current.opacity, mutableListOf(point))
        showDraft(); return true
    }
    /** Each real event is mapped with the DOWN camera. No thinning or predicted
     * samples; exceeding the cap cancels this gesture before publication. */
    public fun append(screenPoint: Point) {
        val value = gesture ?: return
        if (value.admission != admission || !finite(screenPoint)) { cancelGesture(); return }
        val point = runCatching { core.mapPoints(value.camera, true, listOf(screenPoint)).singleOrNull() }.getOrNull()
        if (point == null || !finite(point)) { cancelGesture(); return }
        if (value.tool == SelectionTool.Rectangle) {
            if (value.points.size == 1) value.points += point else value.points[1] = point
        } else if (value.points.last() != point) {
            if (value.points.size == 16_384) {
                cancelGesture(); mutableState.value = state.value.copy(message = "This gesture reached 16,384 real samples. Use a shorter gesture; nothing was changed."); return
            }
            value.points += point
        }
        showDraft()
    }
    public fun finish(screenPoint: Point? = null): Boolean {
        if (screenPoint != null) append(screenPoint)
        val value = gesture ?: return false
        gesture = null; mutableState.value = state.value.copy(draft = null)
        if (value.admission != admission) return false
        val points = value.points.toList()
        val action = when (value.tool) {
            SelectionTool.Rectangle -> {
                val first = points.first(); val last = points.last()
                val rectangle = Rect(min(first.x, last.x), min(first.y, last.y), kotlin.math.abs(last.x - first.x), kotlin.math.abs(last.y - first.y))
                if (rectangle.width == 0.0 || rectangle.height == 0.0) return false
                SelectionOperation.Rectangle(rectangle, value.combine)
            }
            SelectionTool.Lasso -> {
                if (points.size < 3) { mutableState.value = state.value.copy(message = "A lasso needs at least three points."); return false }
                SelectionOperation.Lasso(points, value.combine)
            }
            SelectionTool.Paint, SelectionTool.MaskEraser -> SelectionOperation.Paint(points, value.radius, value.opacity, value.combine)
        }
        return submit(value.document, value.target, action)
    }
    public fun cancelGesture() { gesture = null; mutableState.value = state.value.copy(draft = null) }
    private fun showDraft() {
        val value = gesture ?: return
        mutableState.value = state.value.copy(draft = SelectionDraft(value.tool, value.points.toList(), value.radius))
    }
    public fun refine(value: SelectionRefinement): Boolean {
        if (!state.value.canRefine) return false
        val doc = state.value.document ?: return false; val selection = state.value.selection ?: return false
        val radius = state.value.refinementRadius
        val action = when (value) {
            SelectionRefinement.Invert -> SelectionOperation.Invert
            SelectionRefinement.Expand -> SelectionOperation.Expand(radius)
            SelectionRefinement.Shrink -> SelectionOperation.Shrink(radius)
            SelectionRefinement.Feather -> SelectionOperation.Feather(radius)
        }
        return submit(doc, SelectionTarget.Existing(selection.selection), action)
    }
    private fun submit(document: SelectionDocument, target: SelectionTarget, action: SelectionOperation): Boolean {
        if (operation?.isActive == true || state.value.busy || state.value.document != document) return false
        val facade = api ?: return false; val job = owner?.takeIf { it.isActive } ?: return false; val currentAttachment = attachment
        val time = nowMs(); if (time < 0) return false
        val edit = SelectionEdit(document.binding, core.newId(time.toULong()), document.revision.deviceId,
            document.revision.nextLamport, time, target, action)
        val objectId = when (target) { is SelectionTarget.New -> target.objectId; is SelectionTarget.Existing -> target.selection.objectId }
        val previousObjectId = selectedId
        mutableState.value = state.value.copy(busy = true, preview = null, message = "Saving selection…")
        // Install the receipt/refresh finally block before the first suspension,
        // including cancellation immediately after accepting the UI gesture.
        operation = CoroutineScope(scope.coroutineContext + job).launch(start = CoroutineStart.UNDISPATCHED) {
            var receipt: SelectionReceipt? = null
            selectedId = objectId
            try {
                receipt = calls.withLock { facade.edit(edit) }
                // Receipt ownership is recorded before any suspending editor refresh.
                if (attachment == currentAttachment) selectedId = objectId
            } catch (error: CancellationException) { throw error }
            catch (error: Exception) { if (attachment == currentAttachment) {
                if (error is SelectionFailure && error.kind in setOf(SelectionFailureKind.Invalid, SelectionFailureKind.Conflict,
                    SelectionFailureKind.ReusedTransaction, SelectionFailureKind.Locked, SelectionFailureKind.Unsupported,
                    SelectionFailureKind.Memory, SelectionFailureKind.Limit, SelectionFailureKind.EncodedLimit)) selectedId = previousObjectId
                setFailure(error)
            } }
            finally {
                withContext(NonCancellable) {
                    if (attachment == currentAttachment && !detaching) {
                        try { afterMutation(receipt) }
                        catch (error: Exception) { setFailure(error) }
                        finally {
                            mutableState.value = state.value.copy(busy = false, message = if (receipt != null) "Selection saved." else state.value.message)
                            wakes?.trySend(Unit)
                        }
                    }
                }
            }
        }
        return true
    }
    public fun cancelOperation() {
        cancelGesture(); operation?.cancel()
        mutableState.value = state.value.copy(message = "Cancelling; checking whether the selection was already saved…")
    }

    public fun saveTicket(kind: SelectionExportKind, cropToSelection: Boolean): SelectionSaveTicket? {
        val current = state.value; val doc = current.document ?: return null; val selection = current.selection ?: return null
        if (current.busy || current.loading || owner?.isActive != true || selection.nonzeroBounds == null) return null
        return SelectionSaveTicket(attachment, admission, doc.binding, selection.selection, kind,
            if (cropToSelection) selection.nonzeroBounds else null)
    }
    public fun matches(ticket: SelectionSaveTicket): Boolean = ticket.attachment == attachment && ticket.admission == admission &&
        state.value.document?.binding == ticket.binding && state.value.selection?.selection == ticket.selection

    /** Caller owns staging and awaits this before cleanup. This returns only a
     * completed native receipt; destination publication belongs to the platform. */
    public suspend fun export(ticket: SelectionSaveTicket, workDirectory: String, outputPath: String): SelectionExportReceipt {
        if (!matches(ticket) || state.value.busy || owner?.isActive != true) throw SelectionFailure(SelectionFailureKind.Conflict)
        val facade = api ?: throw SelectionFailure(SelectionFailureKind.Closed)
        val source = state.value.document ?: throw SelectionFailure(SelectionFailureKind.Closed)
        val expectedRegion = ticket.region ?: SelectionRegion(0u, 0u, source.width, source.height)
        val expectedDepth = if (ticket.kind == SelectionExportKind.Mask) 8u else source.sourceBitDepth
        // Export joins the attachment owner for detach, without adopting the
        // caller's scope or letting cancellation bypass producer settlement.
        val job = currentCoroutineContext()[Job] ?: throw SelectionFailure(SelectionFailureKind.Closed)
        if (operation?.isActive == true) throw SelectionFailure(SelectionFailureKind.Backpressure)
        operation = job; mutableState.value = state.value.copy(busy = true, message = "Preparing full-resolution PNG…")
        try {
            val result = calls.withLock {
                if (!matches(ticket)) throw SelectionFailure(SelectionFailureKind.Conflict)
                facade.exportFile(SelectionExportOptions(ticket.binding, ticket.selection, ticket.kind, ticket.region, workDirectory, outputPath))
            }
            currentCoroutineContext().ensureActive()
            if (!matches(ticket) || result.binding != ticket.binding || result.selection != ticket.selection || result.kind != ticket.kind ||
                !result.binding.matches(result.revision, ticket.binding.documentId) || result.encodedBytes == 0uL || result.encodedBytes > 64uL * 1024uL * 1024uL ||
                result.region != expectedRegion || result.outputBitDepth.toUInt() != expectedDepth ||
                result.blake3.length != 64 || result.blake3.any { it !in '0'..'9' && it !in 'a'..'f' }) throw SelectionFailure(SelectionFailureKind.Conflict)
            return result
        } finally {
            if (operation === job) operation = null
            mutableState.value = state.value.copy(busy = false)
        }
    }

    /** Await on the UI dispatcher before closing/replacing the project. */
    public suspend fun detach(): Unit = withContext(NonCancellable) {
        detaching = true
        admission++; previewAdmission++; gesture = null
        val jobs = listOfNotNull(operation, reader).distinct()
        owner?.cancel(); jobs.forEach { it.cancel() }
        jobs.joinAll()
        wakes?.close(); wakes = null; owner = null; reader = null; operation = null
        api = null; project = null; displayed = null; documentId = null; selectedId = null; camera = null
        mutableState.value = SelectionUiState(active = state.value.active)
        detaching = false
    }

    private suspend fun reload() {
        if (!state.value.active) {
            mutableState.value = state.value.copy(document = null, selection = null, preview = null, loading = false)
            return
        }
        val ownerProject = project ?: return; val display = displayed ?: return
        val stamp = admission; val frame = previewAdmission; val id = selectedId; val view = camera
        val facade = api ?: capability(ownerProject).also { api = it }
        val loaded = calls.withLock {
            if (stamp != admission || !state.value.active || project !== ownerProject) return@withLock null
            val document = facade.document(display.documentId)
            if (!document.binding.matches(display.render.revision, display.documentId)) return@withLock null
            val selected = id?.takeIf { state.value.active }?.let { facade.snapshot(document.binding, it) }
            val window = if (selected != null && view != null) selectionPreviewRegions(core, view, document.width, document.height) else emptyList<SelectionRegion>() to false
            val tiles = if (selected?.visible == true && window.first.isNotEmpty()) facade.tiles(document.binding, selected.selection, window.first) else null
            if (tiles != null) validateTiles(tiles, document.binding, selected!!.selection, window.first)
            Triple(document, selected, tiles?.let { SelectionPreview(it.binding, it.selection, it.tiles, window.second) })
        }
        if (stamp != admission || display !== displayed || id != selectedId || !state.value.active || project !== ownerProject) return
        if (loaded == null) {
            mutableState.value = state.value.copy(loading = true, message = "Waiting for the current document revision…"); return
        }
        mutableState.value = state.value.copy(document = loaded.first, selection = loaded.second,
            preview = if (frame == previewAdmission) loaded.third else null, loading = false)
    }
    private fun setFailure(error: Exception) {
        val message = when (error) {
            is SelectionFailure -> when (error.kind) {
                SelectionFailureKind.Conflict, SelectionFailureKind.ReusedTransaction -> "The document or selection changed. Refresh, then draw a new gesture."
                SelectionFailureKind.Memory, SelectionFailureKind.Limit, SelectionFailureKind.EncodedLimit -> "This selection exceeds the current size or work budget. No resizing was applied."
                SelectionFailureKind.Locked -> "The selection or its layer is locked."
                SelectionFailureKind.Unsupported, SelectionFailureKind.Invalid -> "Choose a raster selection or create a new one. This operation is not supported for that object."
                SelectionFailureKind.Cancelled -> "Cancelled. The document refresh shows any operation that had already saved."
                else -> "The selection operation could not finish (${error.kind.name}). The original image is preserved."
            }
            else -> "The selection operation could not finish. The original image is preserved."
        }
        mutableState.value = state.value.copy(loading = false, message = message)
    }
}

private fun finite(point: Point): Boolean = point.x.isFinite() && point.y.isFinite()
private fun validCamera(camera: Camera): Boolean = finite(camera.center) && camera.scale.isFinite() && camera.scale > 0 &&
    camera.rotation.isFinite() && camera.viewportWidth.isFinite() && camera.viewportHeight.isFinite() &&
    camera.viewportWidth > 0 && camera.viewportHeight > 0
private fun sameRevision(a: ProjectInfo, b: ProjectInfo): Boolean = a.projectId == b.projectId && a.hostSeq == b.hostSeq &&
    a.stateHash == b.stateHash && a.nextLamport == b.nextLamport

/** Exact full-resolution coverage in at most sixteen 256px tiles. At a zoomed-
 * out view larger than 1024px, show the central bounded window explicitly rather
 * than silently downsampling the mask or allocating a document-sized overlay. */
public fun selectionPreviewRegions(core: WorkbenchCore, camera: Camera, width: UInt, height: UInt): Pair<List<SelectionRegion>, Boolean> {
    if (!validCamera(camera) || width == 0u || height == 0u) return emptyList<SelectionRegion>() to false
    val corners = core.mapPoints(camera, true, listOf(Point(0.0, 0.0), Point(camera.viewportWidth, 0.0),
        Point(camera.viewportWidth, camera.viewportHeight), Point(0.0, camera.viewportHeight)))
    if (corners.size != 4 || corners.any { !finite(it) }) return emptyList<SelectionRegion>() to false
    val x0 = floor(corners.minOf { it.x }.coerceIn(0.0, width.toDouble())).toLong()
    val y0 = floor(corners.minOf { it.y }.coerceIn(0.0, height.toDouble())).toLong()
    val x1 = ceil(corners.maxOf { it.x }.coerceIn(0.0, width.toDouble())).toLong()
    val y1 = ceil(corners.maxOf { it.y }.coerceIn(0.0, height.toDouble())).toLong()
    if (x1 <= x0 || y1 <= y0) return emptyList<SelectionRegion>() to false
    val left = x0 / 256; val top = y0 / 256; val right = (x1 - 1) / 256; val bottom = (y1 - 1) / 256
    val columns = min(4L, right - left + 1); val rows = min(4L, bottom - top + 1)
    val startX = max(left, ((left + right + 1 - columns) / 2)); val startY = max(top, ((top + bottom + 1 - rows) / 2))
    val result = mutableListOf<SelectionRegion>()
    for (y in startY until startY + rows) for (x in startX until startX + columns) {
        val px = x * 256; val py = y * 256
        result += SelectionRegion(px.toUInt(), py.toUInt(), min(256L, width.toLong() - px).toUInt(), min(256L, height.toLong() - py).toUInt())
    }
    return result to (columns != right - left + 1 || rows != bottom - top + 1)
}
private fun validateTiles(value: SelectionTiles, binding: SelectionBinding, selection: SelectionVersion, regions: List<SelectionRegion>) {
    if (value.binding != binding || value.selection != selection || value.tiles.size != regions.size || value.tiles.size > 16 ||
        value.tiles.zip(regions).any { (tile, expected) -> tile.region != expected || tile.coverage.size.toLong() != expected.width.toLong() * expected.height.toLong() })
        throw SelectionFailure(SelectionFailureKind.Corrupt)
}
