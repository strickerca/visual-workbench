package com.visualworkbench.shared

public enum class InstructionRole { None, Change, Preserve, Reference, Explain }
/** Adapter provenance, not evidence of an OS recognizer or permission. */
public enum class InstructionEntryMethod { PcKeyboard, PhoneKeyboard, Voice, Handwriting }
public data class InstructionRow(public val instructionId: String, public val targetIds: List<String>, public val role: InstructionRole, public val text: String, public val entryMethod: InstructionEntryMethod, public val language: String, public val updatedAtMs: Long, public val detached: Boolean)
public data class MarkerRow(public val objectId: String, public val instructionId: String?, public val number: UInt, public val pointDocument: Point, public val boundsDocument: Rect?, public val elementEids: List<String>, public val hidden: Boolean, public val layerVisible: Boolean)
public data class InstructionDocument(public val binding: WorkflowBinding, public val instructions: List<InstructionRow>, public val markers: List<MarkerRow>, public val needsReconciliation: Boolean)
public data class InstructionProjection(public val binding: WorkflowBinding, public val json: ByteArray, public val promptFragment: String)
public sealed interface InstructionCommand {
    public data class PlaceMarker(public val objectId: String, public val instructionId: String, public val layerId: String, public val point: Point, public val bounds: Rect?, public val elementEids: List<String>, public val style: ObjectStyle, public val role: InstructionRole, public val text: String, public val entryMethod: InstructionEntryMethod, public val language: String = "") : InstructionCommand
    public data class SetInstruction(public val instructionId: String, public val targetIds: List<String>, public val role: InstructionRole, public val text: String, public val entryMethod: InstructionEntryMethod, public val language: String = "") : InstructionCommand
    public data class DeleteMarker(public val objectId: String) : InstructionCommand
    public data class DeleteInstruction(public val instructionId: String) : InstructionCommand
}
public enum class DraftUpdateStatus { Applied, Duplicate, Stale }
public data class InstructionDraftValue(public val sessionId: String, public val instructionId: String, public val binding: WorkflowBinding, public val text: String, public val entryMethod: InstructionEntryMethod)
/** Partial/final recognizer callbacks only update a local literal draft. The UI
 * must explicitly prepare and commit. Keep one session ID/focus generation for
 * one field, close on focus disposal, and never broadcast recognition prediction.
 * Stale preparation preserves text for explicit refresh/reapply. */
public interface WorkbenchInstructionDraft {
    public fun value(): InstructionDraftValue
    public fun update(sessionId: String, focusGeneration: ULong, sequence: ULong, text: String): DraftUpdateStatus
    public suspend fun prepare(sessionId: String, focusGeneration: ULong, metadata: WorkflowMetadata): WorkbenchWorkflowPlan
    public fun close()
}
public interface WorkbenchInstructions {
    public suspend fun document(documentId: String, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): InstructionDocument
    public suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: InstructionCommand, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchWorkflowPlan
    public suspend fun beginDraft(binding: WorkflowBinding, instructionId: String, sessionId: String, focusGeneration: ULong, entryMethod: InstructionEntryMethod, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchInstructionDraft
    public suspend fun export(binding: WorkflowBinding, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): InstructionProjection
}
/** Additive capability; the project retains lifetime authority. Close all owned
 * drafts/plans when the screen/project closes. No focus transport is implied. */
public expect fun instructionWorkflows(project: WorkbenchProject): WorkbenchInstructions
