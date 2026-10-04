package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

public data class SemanticEditorState(
    public val binding: WorkflowBinding? = null, public val catalog: SemanticCatalog? = null,
    public val snapshotId: String? = null, public val document: SemanticDocument? = null,
    public val selectedEid: String? = null, public val quotedReference: String? = null,
    public val snapping: Boolean = true, public val busy: Boolean = false, public val message: String? = null,
)
public data class SemanticPlacement(public val binding: WorkflowBinding, public val snapshotId: String?, public val point: Point,
    public val bounds: Rect?, public val elementEids: List<String>, public val style: ObjectStyle)
/** UI-dispatcher owned, borrowed project, one joined operation. All canonical
 * geometry/quoting comes from the native capability. The owner passes an actual
 * D-to-viewport PHYSICAL-pixel matrix (no additional Compose-density scaling).
 * detach must settle before project.close; it is safe while gate is held. */
public class SemanticEditor(
    private val scope: CoroutineScope, private val gate: Mutex,
    private val context: () -> InstructionAttachment?, private val editable: () -> Boolean,
    private val metadata: suspend (WorkbenchProject) -> WorkflowMetadata,
    private val committed: suspend (WorkflowReceipt) -> Unit,
    private val placeMarker: (SemanticPlacement) -> Unit,
    private val ui: (WorkbenchProject) -> WorkbenchSemanticUi = ::semanticUi,
    private val semantics: (WorkbenchProject) -> WorkbenchSemantics = ::semanticWorkflows,
    private val interactionChanged: () -> Unit = {},
) {
    private val mutable: MutableStateFlow<SemanticEditorState> = MutableStateFlow(SemanticEditorState())
    public val state: StateFlow<SemanticEditorState> = mutable
    private var work: Job? = null
    private var generation: ULong = 0uL
    private var saving: Boolean = false
    private var reload: Boolean = false
    private var detached: Boolean = false
    private fun changed(): Unit { check(generation < ULong.MAX_VALUE); generation++ }
    private fun same(a: InstructionAttachment): Boolean = context()?.let { it.project === a.project && it.binding == a.binding } == true
    public fun message(value: String?): Unit { mutable.value = mutable.value.copy(message = value) }
    public fun snapping(value: Boolean): Unit {
        if (state.value.busy) return
        changed(); mutable.value = mutable.value.copy(snapping = value,
            message = if (value && state.value.snapshotId == null) "Select a stored snapshot before snapping. Turn snapping off for an unreferenced marker." else null)
    }
    private fun launch(edit: Boolean = false, block: suspend (InstructionAttachment, ULong) -> Unit): Unit {
        if (detached || !scope.isActive) return
        if (work != null) { message("Finish or cancel the current semantic operation first."); return }
        val a = context() ?: run { message("Open a captured document first."); return }
        if (edit && !editable()) { message("Close other editors and save or discard the instruction draft first."); return }
        val ticket = generation
        mutable.value = mutable.value.copy(busy = true, message = null)
        interactionChanged()
        val next = scope.launch(start = CoroutineStart.LAZY) {
            try { gate.withLock {
                currentCoroutineContext().ensureActive()
                if (ticket != generation || !same(a) || (edit && !editable())) throw WorkflowFailure(WorkflowFailureKind.Stale)
                block(a, ticket)
            } }
            catch (error: CancellationException) { throw error }
            catch (error: Exception) { if (ticket == generation) message(semanticFailureMessage(error)) }
        }
        work = next
        // A LAZY job cancelled after start but before dispatch never enters its
        // body. Completion owns retirement, including that zero-body path.
        // Keep the exact job until settlement; isActive is false while a
        // cancelled operation may still be closing a native plan.
        next.invokeOnCompletion { error ->
            if (work === next) {
                if (error is CancellationException && ticket == generation && !detached)
                    message("Semantic operation cancelled. Inspect the current revision before retrying.")
                work = null; saving = false; mutable.value = mutable.value.copy(busy = false)
                interactionChanged()
                if (reload && !detached) { reload = false; refresh() }
            }
        }
        next.start()
    }
    private suspend fun admit(a: InstructionAttachment, ticket: ULong): Unit {
        currentCoroutineContext().ensureActive()
        if (ticket != generation || !same(a) || detached) throw WorkflowFailure(WorkflowFailureKind.Stale)
    }
    /** Called after every actual render/revision publication, including offline
     * same-host-sequence changes. Retains only an explicitly chosen snapshot ID
     * in the same document; its decoded payload is revalidated before reuse. */
    public fun refresh(): Unit {
        if (detached) return
        val binding = context()?.binding ?: return
        if (state.value.binding != binding) {
            changed()
            val prior = state.value.binding
            val sameDocument = prior?.projectId == binding.projectId && prior.documentId == binding.documentId
            mutable.value = SemanticEditorState(binding = binding, snapshotId = if (sameDocument) state.value.snapshotId else null,
                snapping = if (sameDocument) state.value.snapping else true, busy = state.value.busy)
            if (!saving) work?.cancel()
        }
        if (work != null) { reload = true; return }
        launch { a, ticket ->
            val catalog = ui(a.project).catalog(a.binding)
            admit(a, ticket)
            if (catalog.binding != a.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            mutable.value = mutable.value.copy(binding = a.binding, catalog = catalog, snapping = if (catalog.isCapture) state.value.snapping else false,
                message = if (!catalog.isCapture) "This document has no capture provenance. Capture a window or use the accessibility capture action; no semantic data was invented."
                    else if (catalog.snapshots.isEmpty()) "No stored semantic snapshot is available. Capture with the semantic provider enabled, or turn snapping off." else null)
            state.value.snapshotId?.let { chosen -> load(a, ticket, chosen) }
        }
    }
    public fun nextPage(): Unit {
        val cursor = state.value.catalog?.next ?: return
        launch { a, ticket ->
            if (cursor.binding != a.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            val page = ui(a.project).catalog(a.binding, cursor)
            admit(a, ticket); if (page.binding != a.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            mutable.value = mutable.value.copy(catalog = page) // bounded page, not a growing history
        }
    }
    public fun selectSnapshot(id: String): Unit {
        if (state.value.busy) return
        if (state.value.catalog?.snapshots?.none { it.snapshotId == id } != false) { message("Select a snapshot from the current stored page."); return }
        changed(); mutable.value = mutable.value.copy(snapshotId = id, document = null, selectedEid = null, quotedReference = null)
        launch { a, ticket -> load(a, ticket, id) }
    }
    private suspend fun load(a: InstructionAttachment, ticket: ULong, id: String): Unit {
        val doc = semantics(a.project).document(a.binding, id)
        admit(a, ticket)
        if (doc.binding != a.binding || doc.snapshotId != id || doc.elements.size > 4096) throw WorkflowFailure(WorkflowFailureKind.Stale)
        mutable.value = mutable.value.copy(document = doc, selectedEid = null, quotedReference = null)
    }
    public fun selectElement(eid: String): Unit {
        val doc = state.value.document ?: run { message("Select a stored snapshot first."); return }
        if (doc.elements.none { it.eid == eid }) { message("That element is unavailable in the selected snapshot."); return }
        launch { a, ticket ->
            if (doc.binding != a.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            val quoted = semantics(a.project).export(a.binding, doc.snapshotId, listOf(eid))
            admit(a, ticket)
            if (quoted.binding != a.binding || quoted.snapshotId != doc.snapshotId || quoted.references.singleOrNull()?.eid != eid) throw WorkflowFailure(WorkflowFailureKind.Stale)
            mutable.value = mutable.value.copy(selectedEid = eid, quotedReference = quoted.quotedPromptData)
        }
    }
    public fun place(point: Point, style: ObjectStyle, documentToPhysical: Transform, bounds: Rect? = null, invertSnapping: Boolean = false): Unit {
        val selected = state.value.document
        val snap = state.value.snapping xor invertSnapping
        if (snap && selected == null) { message("Select a stored snapshot before snapping, or explicitly turn snapping off."); return }
        launch(edit = true) { a, ticket ->
            if (snap && selected?.binding != a.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            val hit = if (snap && selected != null) semantics(a.project).snap(a.binding, selected.snapshotId,
                bounds?.let { SemanticSnapQuery.BoxQuery(it) } ?: SemanticSnapQuery.PointQuery(point), documentToPhysical) else null
            admit(a, ticket)
            if (!editable()) throw WorkflowFailure(WorkflowFailureKind.Stale)
            if (hit != null && (hit.binding != a.binding || hit.snapshotId != selected?.snapshotId || !hit.distanceScreenPixels.isFinite() || hit.distanceScreenPixels !in 0.0..12.0)) throw WorkflowFailure(WorkflowFailureKind.Invalid)
            val box = hit?.boundsDocument ?: bounds
            val marker = hit?.boundsDocument?.let { Point(it.x + it.width / 2, it.y + it.height / 2) } ?: point
            placeMarker(SemanticPlacement(a.binding, if (snap) selected?.snapshotId else null, marker, box, hit?.let { listOf(it.eid) }.orEmpty(), style))
        }
    }
    public fun bindMarker(objectId: String, clear: Boolean = false): Unit {
        val doc = state.value.document ?: run { message("Select a stored snapshot first."); return }
        val eid = state.value.selectedEid
        if (!clear && eid == null) { message("Select a captured element first."); return }
        launch(edit = true) { a, ticket ->
            if (a.binding != doc.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            var plan: WorkbenchWorkflowPlan? = null
            try {
                val meta = metadata(a.project); admit(a, ticket)
                plan = ui(a.project).prepareReferences(a.binding, doc.snapshotId, objectId, if (clear) emptyList() else listOfNotNull(eid), meta)
                admit(a, ticket)
                if (!editable()) throw WorkflowFailure(WorkflowFailureKind.Stale)
                saving = true
                val receipt = plan.commit()
                withContext(NonCancellable) { committed(receipt) }
                reload = true
                message(if (clear) "Element references cleared. The marker and instruction are preserved." else "Element reference saved with the marker. Undo restores the previous references.")
            } finally { plan?.close() }
        }
    }
    public fun cancel(): Unit { work?.cancel() }
    public fun resume(): Unit { check(work == null); detached = false; refresh() }
    /** Replacement can fail before its normal closeCurrent/discardExport path.
     * Always join old work before resuming, even when the mutation gate is held. */
    public suspend fun resumeAfterReplacement(): Unit { detach(); resume() }
    public suspend fun detach(): Unit {
        detached = true; changed(); reload = false
        withContext(NonCancellable) { work?.cancelAndJoin(); work = null; mutable.value = SemanticEditorState(); interactionChanged() }
    }
}
public fun semanticFailureMessage(error: Exception): String = when ((error as? WorkflowFailure)?.kind) {
    WorkflowFailureKind.Stale -> "The document, capture or revision changed. Refresh and explicitly select the snapshot again."
    WorkflowFailureKind.Missing -> "The stored snapshot or element is missing. No replacement was synthesized."
    WorkflowFailureKind.Locked -> "The marker or its layer is locked. Unlock it before changing references."
    WorkflowFailureKind.Reconciliation -> "This marker needs its instruction link repaired before references can change."
    WorkflowFailureKind.Memory, WorkflowFailureKind.Limit -> "Semantic data exceeds the bounded workspace or element limit. Saved content is preserved."
    WorkflowFailureKind.Backpressure -> "The core is busy. Finish current work, then retry explicitly."
    WorkflowFailureKind.Closed -> "The project closed. Reopen it to inspect saved snapshots."
    else -> "Semantic data could not be validated. Refresh the document or capture a new snapshot."
}
