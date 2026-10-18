package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.DocumentControlCommand as NCommand
import com.visualworkbench.bindings.core.DocumentControlTargets as NTargets
import com.visualworkbench.bindings.core.LayerControlUpdate as NUpdate

public actual fun documentControlWorkflows(project: WorkbenchProject): WorkbenchDocumentControls =
    NativeDocumentControls(nativeProjectHandle(project))

private fun DocumentControlTargets.native(): NTargets = when (this) {
    is DocumentControlTargets.Objects -> NTargets.Objects(objectIds)
    is DocumentControlTargets.Group -> NTargets.Group(groupId)
}
private fun DocumentControlCommand.native(): NCommand = when (this) {
    is DocumentControlCommand.CreateAnnotationLayer -> NCommand.CreateAnnotationLayer(layerId, name)
    is DocumentControlCommand.UpdateLayer -> update.let { NCommand.UpdateLayer(NUpdate(it.layerId,it.name,it.visible,it.locked,it.opacity,it.blend)) }
    is DocumentControlCommand.PlaceLayer -> NCommand.PlaceLayer(layerId,beforeLayerId)
    is DocumentControlCommand.DeleteEmptyLayer -> NCommand.DeleteEmptyLayer(layerId)
    is DocumentControlCommand.Group -> NCommand.Group(groupId,memberIds)
    is DocumentControlCommand.Ungroup -> NCommand.Ungroup(groupId)
    is DocumentControlCommand.SetObjectLocks -> NCommand.SetObjectLocks(targets.native(),locked)
    is DocumentControlCommand.SetObjectHidden -> NCommand.SetObjectHidden(targets.native(),hidden)
    is DocumentControlCommand.MoveObjects -> NCommand.MoveObjects(targets.native(),layerId)
    is DocumentControlCommand.RenameDocument -> NCommand.RenameDocument(title)
    is DocumentControlCommand.CreateDocumentFromSource -> NCommand.CreateDocumentFromSource(documentId,layerId,title)
}
private class NativeDocumentControls(private val handle: ProjectSession) : WorkbenchDocumentControls {
    override suspend fun snapshot(documentId: String, memoryBudgetBytes: ULong): DocumentControlSnapshot {
        if (documentId.length != 36) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        return settledWorkflow { cancellation ->
            val value = handle.documentControlsSnapshot(documentId,memoryBudgetBytes,cancellation)
            DocumentControlSnapshot(workflowBinding(value.binding),workflowInfo(value.revision),
                value.documents.map { DocumentControlRow(it.documentId,it.title,it.kind,it.width,it.height) },
                value.layers.map { LayerControlRow(it.layerId,it.name,it.kind,it.orderKey,it.visible,it.locked,it.opacity,it.blend,it.objectCount) },
                value.objects.map { ObjectControlRow(it.objectId,it.layerId,it.groupId,it.orderKey,it.kind,it.hidden,it.locked,
                    it.transform.let { a -> Transform(a.a,a.b,a.c,a.d,a.e,a.f) }) },
                value.groups.map { GroupControlRow(it.groupId,it.parentId) })
        }
    }
    override suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: DocumentControlCommand,
        memoryBudgetBytes: ULong): WorkbenchWorkflowPlan {
        val owned = command.ownedDocumentControl().native()
        val expected = workflowBinding(binding)
        val author = workflowMetadata(metadata)
        val plan = settledWorkflow(release = ::closeWorkflowPlan) { cancellation ->
            handle.prepareDocumentControl(expected,author,owned,memoryBudgetBytes,cancellation)
        }
        return wrapWorkflowPlan(plan)
    }
}
