package com.visualworkbench.shared

import com.visualworkbench.bindings.core.SelectionRegion as NRegion

public actual suspend fun imageOutputExportGeometry(
    project: WorkbenchProject, binding: WorkflowBinding, sourceRegion: SelectionRegion?,
    memoryBudgetBytes: ULong,
): ImageOutputExportGeometry {
    val expected = workflowBinding(binding)
    return try {
        withNativeGiantProject(project) { handle ->
            settledWorkflow { cancellation ->
                val value = handle.imageOutputExportGeometry(expected,
                    sourceRegion?.let { NRegion(it.x, it.y, it.width, it.height) }, memoryBudgetBytes, cancellation)
                ImageOutputExportGeometry(workflowBinding(value.binding), value.sourceAssetId,
                    value.sourceWidth, value.sourceHeight, value.outputWidth, value.outputHeight,
                    value.region?.let { SelectionRegion(it.x, it.y, it.width, it.height) },
                    value.encodedWidth, value.encodedHeight)
            }
        }
    } catch (error: GiantImageFailure) {
        throw WorkflowFailure(if (error.kind == GiantImageFailureKind.Closed) WorkflowFailureKind.Closed
            else WorkflowFailureKind.Invalid, cause = error)
    }
}
