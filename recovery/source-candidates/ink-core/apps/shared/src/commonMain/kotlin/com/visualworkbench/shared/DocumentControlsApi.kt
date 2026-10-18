package com.visualworkbench.shared

public data class DocumentControlRow(public val documentId: String, public val title: String,
    public val kind: String, public val width: UInt, public val height: UInt)
public data class LayerControlRow(public val layerId: String, public val name: String, public val kind: String,
    public val orderKey: String, public val visible: Boolean, public val locked: Boolean, public val opacity: Double,
    public val blend: String, public val objectCount: UInt)
public data class ObjectControlRow(public val objectId: String, public val layerId: String, public val groupId: String?,
    public val orderKey: String, public val kind: String, public val hidden: Boolean, public val locked: Boolean,
    public val transform: Transform)
public data class GroupControlRow(public val groupId: String, public val parentId: String?)
public data class DocumentControlSnapshot(public val binding: WorkflowBinding, public val revision: ProjectInfo,
    public val documents: List<DocumentControlRow>, public val layers: List<LayerControlRow>,
    public val objects: List<ObjectControlRow>, public val groups: List<GroupControlRow>)
public data class LayerControlUpdate(public val layerId: String, public val name: String? = null,
    public val visible: Boolean? = null, public val locked: Boolean? = null, public val opacity: Double? = null,
    public val blend: String? = null)
public sealed interface DocumentControlTargets {
    public data class Objects(public val objectIds: List<String>) : DocumentControlTargets
    public data class Group(public val groupId: String) : DocumentControlTargets
}
public sealed interface DocumentControlCommand {
    public data class CreateAnnotationLayer(public val layerId: String, public val name: String) : DocumentControlCommand
    public data class UpdateLayer(public val update: LayerControlUpdate) : DocumentControlCommand
    /** Back to front. null places at the front. Tied/exhausted intervals refuse. */
    public data class PlaceLayer(public val layerId: String, public val beforeLayerId: String?) : DocumentControlCommand
    public data class DeleteEmptyLayer(public val layerId: String) : DocumentControlCommand
    public data class Group(public val groupId: String, public val memberIds: List<String>) : DocumentControlCommand
    public data class Ungroup(public val groupId: String) : DocumentControlCommand
    public data class SetObjectLocks(public val targets: DocumentControlTargets, public val locked: Boolean) : DocumentControlCommand
    public data class SetObjectHidden(public val targets: DocumentControlTargets, public val hidden: Boolean) : DocumentControlCommand
    public data class MoveObjects(public val targets: DocumentControlTargets, public val layerId: String) : DocumentControlCommand
    public data class RenameDocument(public val title: String) : DocumentControlCommand
    /** New independent annotations over the same immutable original image. */
    public data class CreateDocumentFromSource(public val documentId: String, public val layerId: String,
        public val title: String) : DocumentControlCommand
}

/** Borrowed project capability. It starts no service, owns no project and never
 * edits until a prepared immutable plan is explicitly committed. Complete
 * catalogs refuse their bounds (128 documents/256 layers/4096 objects/256 groups)
 * rather than truncating selection or operation targets. */
public interface WorkbenchDocumentControls {
    public suspend fun snapshot(documentId: String, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): DocumentControlSnapshot
    public suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: DocumentControlCommand,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchWorkflowPlan
}
public expect fun documentControlWorkflows(project: WorkbenchProject): WorkbenchDocumentControls
public fun WorkbenchProject.documentControls(): WorkbenchDocumentControls = documentControlWorkflows(this)

/** Copy caller-owned lists before the first suspension/foreign conversion. */
internal fun DocumentControlCommand.ownedDocumentControl(): DocumentControlCommand {
    fun id(value: String) { if (value.length != 36) throw WorkflowFailure(WorkflowFailureKind.Invalid) }
    fun text(value: String) {
        if (value.isEmpty() || value.length > 4096 || value.any { it.isISOControl() }) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    }
    fun ids(values: List<String>, minimum: Int): List<String> {
        if (values.size !in minimum..256) throw WorkflowFailure(WorkflowFailureKind.Limit)
        values.forEach(::id)
        if (values.distinct().size != values.size) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        return values.toList()
    }
    fun copyTargets(value: DocumentControlTargets): DocumentControlTargets = when (value) {
        is DocumentControlTargets.Objects -> value.copy(objectIds = ids(value.objectIds, 1))
        is DocumentControlTargets.Group -> value.also { id(it.groupId) }
    }
    return when (this) {
        is DocumentControlCommand.CreateAnnotationLayer -> also { id(layerId); text(name) }
        is DocumentControlCommand.UpdateLayer -> also {
            id(update.layerId); update.name?.let(::text)
            if (update.opacity?.let { !it.isFinite() || it !in 0.0..1.0 } == true ||
                update.blend?.let { it != "normal" && it != "multiply" } == true) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        }
        is DocumentControlCommand.PlaceLayer -> also { id(layerId); beforeLayerId?.let(::id) }
        is DocumentControlCommand.DeleteEmptyLayer -> also { id(layerId) }
        is DocumentControlCommand.Group -> { id(groupId); copy(memberIds = ids(memberIds, 2)) }
        is DocumentControlCommand.Ungroup -> also { id(groupId) }
        is DocumentControlCommand.SetObjectLocks -> copy(targets = copyTargets(targets))
        is DocumentControlCommand.SetObjectHidden -> copy(targets = copyTargets(targets))
        is DocumentControlCommand.MoveObjects -> { id(layerId); copy(targets = copyTargets(targets)) }
        is DocumentControlCommand.RenameDocument -> also { text(title) }
        is DocumentControlCommand.CreateDocumentFromSource -> also { id(documentId); id(layerId); text(title) }
    }
}
