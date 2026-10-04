package com.visualworkbench.shared

import com.visualworkbench.bindings.core.SemanticCatalogCursor as NCursor

public actual fun semanticUi(project: WorkbenchProject): WorkbenchSemanticUi = NativeSemanticUi(nativeProjectHandle(project))
private class NativeSemanticUi(private val handle: com.visualworkbench.bindings.core.ProjectSession) : WorkbenchSemanticUi {
    private fun id(value: String) { if (value.length != 36) throw WorkflowFailure(WorkflowFailureKind.Invalid) }
    override suspend fun catalog(binding: WorkflowBinding, cursor: SemanticCatalogCursor?, limit: UInt, memoryBudgetBytes: ULong): SemanticCatalog {
        if (limit !in 1u..32u) throw WorkflowFailure(WorkflowFailureKind.Limit)
        val expected = workflowBinding(binding)
        val after = cursor?.let { id(it.afterSnapshotId); NCursor(workflowBinding(it.binding), it.afterSnapshotId) }
        return settledWorkflow { token -> handle.semanticCatalog(expected, after, limit, memoryBudgetBytes, token).let { value ->
            SemanticCatalog(workflowBinding(value.binding), value.isCapture,
                value.snapshots.map { SemanticSnapshotSummary(it.snapshotId, it.platform, it.frameDeltaMs, it.createdAtMs) },
                value.next?.let { SemanticCatalogCursor(workflowBinding(it.binding), it.afterSnapshotId) })
        } }
    }
    override suspend fun prepareMarker(binding: WorkflowBinding, snapshotId: String, metadata: WorkflowMetadata, command: InstructionCommand.PlaceMarker, memoryBudgetBytes: ULong): WorkbenchWorkflowPlan {
        id(snapshotId)
        if (command.elementEids.size > 64 || command.elementEids.any { it.length > 300 }) throw WorkflowFailure(WorkflowFailureKind.Limit)
        val request = command.native()
        val plan = settledWorkflow(release = ::closeWorkflowPlan) { token -> handle.prepareSemanticMarker(workflowBinding(binding), snapshotId, workflowMetadata(metadata), request, memoryBudgetBytes, token) }
        return wrapWorkflowPlan(plan)
    }
    override suspend fun prepareReferences(binding: WorkflowBinding, snapshotId: String, objectId: String, elementEids: List<String>, metadata: WorkflowMetadata, memoryBudgetBytes: ULong): WorkbenchWorkflowPlan {
        id(snapshotId); id(objectId)
        if (elementEids.size > 64 || elementEids.any { it.length > 300 }) throw WorkflowFailure(WorkflowFailureKind.Limit)
        val ids = elementEids.toList()
        val plan = settledWorkflow(release = ::closeWorkflowPlan) { token -> handle.prepareSemanticReferences(workflowBinding(binding), snapshotId, objectId, ids, workflowMetadata(metadata), memoryBudgetBytes, token) }
        return wrapWorkflowPlan(plan)
    }
}
