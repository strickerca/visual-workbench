package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

public data class InstructionAttachment(public val project: WorkbenchProject, public val document: DocumentSnapshot) {
    public val binding: WorkflowBinding get() = document.render.revision.let {
        WorkflowBinding(it.projectId, document.documentId, it.hostSeq, it.stateHash)
    }
}
public data class InstructionField(
    public val instructionId: String, public val targetIds: List<String>, public val text: String,
    public val role: InstructionRole, public val method: InstructionEntryMethod, public val language: String,
    public val binding: WorkflowBinding, public val sessionId: String, public val generation: ULong,
    public val dirty: Boolean = false, public val stale: Boolean = false, public val existing: Boolean = true,
)
public data class InstructionEditorState(
    public val document: InstructionDocument? = null, public val field: InstructionField? = null,
    public val busy: Boolean = false, public val message: String? = null,
)
/** All methods run on the owning UI dispatcher. The editor borrows the project.
 * Its single operation shares the host edit mutex. detach() must settle before
 * the host closes the project; it is safe while that mutex is already held.
 * Native plans/drafts never cross an unowned dispatcher handoff. */
public class InstructionEditor(
    private val scope: CoroutineScope,
    private val gate: Mutex,
    private val context: () -> InstructionAttachment?,
    private val newId: () -> String,
    private val metadata: suspend (WorkbenchProject) -> WorkflowMetadata,
    private val committed: suspend (WorkflowReceipt) -> Unit,
    public val keyboard: InstructionEntryMethod,
    private val workflows: (WorkbenchProject) -> WorkbenchInstructions = ::instructionWorkflows,
    private val localFocused: (InstructionField) -> Unit = {},
) {
    private val mutable: MutableStateFlow<InstructionEditorState> = MutableStateFlow(InstructionEditorState())
    public val state: StateFlow<InstructionEditorState> = mutable
    private var work: Job? = null
    private var native: WorkbenchInstructionDraft? = null
    private var original: InstructionRow? = null
    private var sequence: ULong = 0uL
    private var generation: ULong = 0uL
    private var lifetime: ULong = 0uL
    private var reload: Boolean = false
    private var detaching: Boolean = false

    public fun message(text: String?): Unit { mutable.value = mutable.value.copy(message = text) }
    public fun hasUnsaved(): Boolean = mutable.value.field?.dirty == true
    /** Normal replacement/quit must call this on the owner UI dispatcher before
     * its first suspension. Failure preserves the field. Success seals every
     * input path until the owner settles detach and publishes its final context. */
    public fun sealIfClean(): Boolean {
        if (detaching || hasUnsaved()) return false
        detaching = true; lifetime++; reload = false
        mutable.value = mutable.value.copy(busy = true)
        work?.cancel()
        return true
    }
    /** Called only after the replacing/closing owner has joined detach and
     * published its final project (which may still be the prior project). */
    public fun resume(): Unit {
        check(work?.isCompleted != false)
        detaching = false
        mutable.value = mutable.value.copy(busy = false)
    }
    private fun same(value: InstructionAttachment): Boolean = context()?.let {
        it.project === value.project && it.binding == value.binding
    } == true
    private fun field(value: InstructionField?): Unit { mutable.value = mutable.value.copy(field = value) }
    private fun releaseDraft(): Unit { val prior = native; native = null; original = null; sequence = 0uL; prior?.close() }
    private fun bump(): ULong { check(generation < ULong.MAX_VALUE); generation++; return generation }
    private fun launch(block: suspend (InstructionAttachment) -> Unit): Unit {
        if (detaching || !scope.isActive) return
        if (work?.isCompleted == false) { message("Finish or cancel the current instruction operation first."); return }
        val attachment = context() ?: run { message("Open a document first."); return }
        val owned = lifetime
        mutable.value = mutable.value.copy(busy = true, message = null)
        // LAZY ensures work is installed before an undispatched completion.
        val next = scope.launch(start = CoroutineStart.LAZY) {
            try { gate.withLock {
                if (owned != lifetime || !same(attachment)) throw WorkflowFailure(WorkflowFailureKind.Stale)
                block(attachment)
            } }
            catch (cancel: CancellationException) { if (owned == lifetime) message("Operation cancelled. Refresh to inspect any already committed edit."); throw cancel }
            catch (error: Exception) { if (owned == lifetime) {
                if (error is WorkflowFailure && error.kind == WorkflowFailureKind.Stale) field(mutable.value.field?.copy(stale = true))
                message(instructionFailureMessage(error))
            } }
        }
        work = next
        // Own cleanup even when cancellation wins before the body dispatches.
        // A cancelled native call retains this slot until all of its cleanup has
        // settled; only this exact completed job may publish idle or a reload.
        next.invokeOnCompletion {
            if (owned == lifetime && work === next && !detaching) {
                work = null; mutable.value = mutable.value.copy(busy = false)
                if (reload) { reload = false; refresh() }
            }
        }
        next.start()
    }
    /** Called after publication of a canonical/optimistic render snapshot. */
    public fun refresh(): Unit {
        if (detaching) return
        val attachment = context() ?: return
        val current = mutable.value.field
        if (current != null && current.binding != attachment.binding) {
            releaseDraft()
            field(current.copy(stale = true))
        }
        if (work?.isCompleted == false) { reload = true; return }
        launch { load(it) }
    }
    private suspend fun load(attachment: InstructionAttachment): InstructionDocument {
        val rows = workflows(attachment.project).document(attachment.binding.documentId)
        if (!same(attachment) || rows.binding != attachment.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
        mutable.value = mutable.value.copy(document = rows)
        return rows
    }
    public fun focusObject(objectId: String): Unit = focus(target = objectId)
    public fun focusGlobal(): Unit = focus(target = null)
    public fun focusInstruction(instructionId: String): Unit = focus(id = instructionId)
    /** A peer focus is a guarded field acquisition, never a text mutation.
     * admit is rechecked after every native handoff and before publication. */
    public fun followInstruction(id: String, expected: WorkflowBinding, admit: () -> Boolean, opened: () -> Unit): Boolean {
        if (detaching || state.value.busy || hasUnsaved() || context()?.binding != expected || !admit()) return false
        focus(id = id, required = expected, announce = false, admit = admit, opened = opened)
        return true
    }
    private fun focus(target: String? = null, id: String? = null, required: WorkflowBinding? = null,
        announce: Boolean = true, admit: () -> Boolean = { true }, opened: () -> Unit = {}): Unit {
        if (detaching || state.value.busy || !admit()) return
        if (required != null && context()?.binding != required) return
        val shown = mutable.value.field
        if (shown != null && (required == null || shown.binding == required) &&
            ((id != null && shown.instructionId == id) || (id == null && shown.targetIds == listOfNotNull(target)))) {
            if (announce) localFocused(shown)
            opened(); return
        }
        if (hasUnsaved()) { message("Save or discard this draft before selecting another instruction."); return }
        launch { attachment ->
            if ((required != null && attachment.binding != required) || !admit()) return@launch
            val rows = load(attachment)
            if (!admit()) return@launch
            val row = if (id != null) rows.instructions.firstOrNull { it.instructionId == id }
                else rows.instructions.firstOrNull { if (target == null) it.targetIds.isEmpty() else target in it.targetIds }
            if (id != null && row == null) throw WorkflowFailure(WorkflowFailureKind.Missing)
            if (target != null && attachment.document.render.items.none { it.objectId == target }) throw WorkflowFailure(WorkflowFailureKind.Missing)
            open(attachment, row, listOfNotNull(target), announce, admit, opened)
        }
    }
    private suspend fun open(attachment: InstructionAttachment, row: InstructionRow?, targets: List<String>,
        announce: Boolean = true, admit: () -> Boolean = { true }, opened: () -> Unit = {}): Unit {
        releaseDraft()
        val value = InstructionField(row?.instructionId ?: newId(), row?.targetIds ?: targets, row?.text ?: "",
            row?.role ?: InstructionRole.None, keyboard, row?.language ?: "", attachment.binding, newId(), bump(), existing = row != null)
        var acquired: WorkbenchInstructionDraft? = null
        try {
            if (row != null) acquired = workflows(attachment.project).beginDraft(value.binding, value.instructionId, value.sessionId, value.generation, value.method)
            currentCoroutineContext().ensureActive()
            if (!same(attachment)) throw WorkflowFailure(WorkflowFailureKind.Stale)
            if (!admit()) return
            native = acquired; acquired = null; original = row; field(value)
            if (announce) localFocused(value)
            opened()
        } finally { acquired?.close() }
    }
    public fun update(text: String, sessionId: String? = state.value.field?.sessionId, generation: ULong? = state.value.field?.generation): Boolean {
        val value = mutable.value.field ?: return false
        if (detaching || value.sessionId != sessionId || value.generation != generation || state.value.busy) return false
        if (!instructionTextFits(text)) { message("Instruction text is limited to 32 KiB of UTF-8. The prior text is preserved."); return false }
        if (sequence == ULong.MAX_VALUE) { message("Reopen this field before continuing."); return false }
        val next = sequence + 1uL
        try { native?.update(value.sessionId, value.generation, next, text) }
        catch (error: WorkflowFailure) {
            if (error.kind != WorkflowFailureKind.Backpressure) { message(instructionFailureMessage(error)); return false }
            // The literal UI text remains authoritative for the next explicit Save.
        }
        sequence = next; field(value.copy(text = text, dirty = true)); return true
    }
    public fun role(value: InstructionRole): Unit { if (!detaching && !state.value.busy) field(state.value.field?.copy(role = value, dirty = true)) }
    public fun method(value: InstructionEntryMethod): Unit {
        if (detaching || state.value.busy) return
        val prior = state.value.field ?: return
        if (prior.method == value) return
        releaseDraft(); field(prior.copy(method = value, sessionId = newId(), generation = bump()))
    }
    /** Reapply is explicit: it refreshes the binding while retaining literal text.
     * No save follows automatically, and missing/deleted targets remain a refusal. */
    public fun reapply(): Unit {
        val prior = state.value.field ?: return
        launch { attachment ->
            val rows = load(attachment)
            val row = rows.instructions.firstOrNull { it.instructionId == prior.instructionId }
            if (prior.existing && row == null) throw WorkflowFailure(WorkflowFailureKind.Missing)
            if (row?.detached == true || prior.targetIds.any { id -> attachment.document.render.items.none { it.objectId == id } }) throw WorkflowFailure(WorkflowFailureKind.Detached)
            releaseDraft(); original = row
            field(prior.copy(binding = attachment.binding, sessionId = newId(), generation = bump(), stale = false, dirty = true))
            message("Draft retained on the refreshed revision. Review it, then Save explicitly.")
        }
    }
    public fun discard(): Unit {
        if (state.value.busy) { message("Cancel the current operation before discarding."); return }
        releaseDraft(); bump(); field(null); message(null)
    }
    public fun cancel(): Unit { work?.cancel() }
    public fun save(): Unit {
        val value = state.value.field ?: return
        if (value.stale) { message("The document changed. Refresh and reapply this draft before saving."); return }
        launch { attachment ->
            if (attachment.binding != value.binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            val info = metadata(attachment.project)
            var plan: WorkbenchWorkflowPlan? = null
            try {
                val old = original
                val draft = native
                plan = if (draft != null && old != null && value.role == old.role && value.targetIds == old.targetIds && value.language == old.language) {
                    if (sequence == ULong.MAX_VALUE) throw WorkflowFailure(WorkflowFailureKind.Limit)
                    sequence++
                    draft.update(value.sessionId, value.generation, sequence, value.text)
                    draft.prepare(value.sessionId, value.generation, info)
                } else workflows(attachment.project).prepare(value.binding, info, InstructionCommand.SetInstruction(
                    value.instructionId, value.targetIds, value.role, value.text, value.method, value.language))
                require(plan.describe().operationCount <= 1024u)
                val receipt = plan.commit()
                // A durable receipt is bookkeeping even if cancellation arrives
                // before the host refresh. The facade has already settled it.
                withContext(NonCancellable) {
                    releaseDraft(); field(value.copy(dirty = false, stale = true, existing = true))
                    committed(receipt)
                    val next = context()
                    if (next?.project === attachment.project && next.binding.hostSeq == receipt.revision.hostSeq && next.binding.stateHash == receipt.revision.stateHash) {
                        field(value.copy(binding = next.binding, dirty = false, stale = false, existing = true, sessionId = newId(), generation = bump()))
                    }
                }
                reload = true; message("Instruction saved.")
            } finally { plan?.close() }
        }
    }
    public fun place(point: Point, style: ObjectStyle, bounds: Rect? = null,
        elementEids: List<String> = emptyList(), semanticSnapshotId: String? = null, requiredBinding: WorkflowBinding? = null): Unit {
        if (elementEids.size > 64 || elementEids.any { it.length > 300 } || (elementEids.isNotEmpty() && semanticSnapshotId == null)) {
            message("Element references require a validated stored snapshot."); return
        }
        val refs = elementEids.toList()
        if (requiredBinding != null && context()?.binding != requiredBinding) { message("The document changed. Place the marker again."); return }
        if (hasUnsaved()) { message("Save or discard the current draft before placing a marker."); return }
        launch { attachment ->
            if (requiredBinding != null && attachment.binding != requiredBinding) throw WorkflowFailure(WorkflowFailureKind.Stale)
            if (!point.x.isFinite() || !point.y.isFinite() || point.x < 0 || point.y < 0 || point.x >= attachment.document.width.toDouble() || point.y >= attachment.document.height.toDouble()) throw WorkflowFailure(WorkflowFailureKind.Invalid)
            val layer = attachment.document.layers.firstOrNull { it.visible && !it.locked && it.opacity > 0 && it.blend == "normal" }
                ?: throw WorkflowFailure(WorkflowFailureKind.Locked)
            val objectId = newId(); val instructionId = newId()
            val command = InstructionCommand.PlaceMarker(objectId, instructionId, layer.id, point, bounds, refs, style,
                InstructionRole.Change, "", keyboard)
            val meta = metadata(attachment.project)
            val plan = if (semanticSnapshotId == null) workflows(attachment.project).prepare(attachment.binding, meta, command)
                else semanticUi(attachment.project).prepareMarker(attachment.binding, semanticSnapshotId, meta, command)
            try {
                val receipt = plan.commit()
                withContext(NonCancellable) { releaseDraft(); field(null); committed(receipt) }
                // Load using the host's newly published binding, then own one field.
                val next = context() ?: throw WorkflowFailure(WorkflowFailureKind.Closed)
                val rows = load(next); open(next, rows.instructions.first { it.instructionId == instructionId }, listOf(objectId))
                message("Marker placed. Enter its instruction.")
            } finally { plan.close() }
        }
    }
    public fun deleteMarker(objectId: String): Unit = remove(InstructionCommand.DeleteMarker(objectId))
    public fun deleteInstruction(instructionId: String): Unit = remove(InstructionCommand.DeleteInstruction(instructionId))
    private fun remove(command: InstructionCommand): Unit {
        if (hasUnsaved()) { message("Save or discard this draft before deleting."); return }
        launch { attachment ->
            val plan = workflows(attachment.project).prepare(attachment.binding, metadata(attachment.project), command)
            try {
                require(plan.describe().operationCount <= 1024u)
                val receipt = plan.commit()
                withContext(NonCancellable) { releaseDraft(); field(null); committed(receipt) }
                reload = true
            } finally { plan.close() }
        }
    }
    public suspend fun detach(): Unit {
        detaching = true; lifetime++; reload = false
        withContext(NonCancellable) { val prior = work; work = null; prior?.cancelAndJoin(); releaseDraft(); mutable.value = InstructionEditorState(busy = true) }
    }
}
/** Rejects rather than truncates. The early UTF-16 bound caps the UTF-8 allocation. */
public fun instructionTextFits(value: String): Boolean = value.length <= 32768 && value.encodeToByteArray().size <= 32768 && '\u0000' !in value
public fun instructionFailureMessage(error: Exception): String = when ((error as? WorkflowFailure)?.kind) {
    WorkflowFailureKind.Stale -> "The document changed. Your draft is retained; review and reapply it before saving."
    WorkflowFailureKind.Locked -> "This instruction affects a locked object or layer. Unlock it before editing."
    WorkflowFailureKind.Detached, WorkflowFailureKind.Missing -> "An instruction target was deleted or is unavailable. The draft is retained for explicit repair."
    WorkflowFailureKind.Reconciliation -> "Marker numbering or links require reconciliation. Saved content was not renumbered automatically."
    WorkflowFailureKind.Memory, WorkflowFailureKind.Limit -> "This instruction operation exceeds its resource limit. Saved content is preserved."
    WorkflowFailureKind.Backpressure -> "The core is busy. Finish current work and try again."
    WorkflowFailureKind.Closed -> "The project closed. Reopen it before editing instructions."
    WorkflowFailureKind.Cancelled -> "Instruction operation cancelled. Refresh to inspect any already committed edit."
    else -> "The instruction could not be saved. The draft is retained; refresh and review before retrying."
}
