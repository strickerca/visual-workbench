package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.VectorEraseOptions as NOptions
import com.visualworkbench.bindings.core.VectorEraseTarget as NTarget

public actual fun WorkbenchProject.vectorEraser(): WorkbenchVectorEraser {
    val handle = try { nativeProjectHandle(this) }
        catch (error: SessionFailure) { throw WorkflowFailure(WorkflowFailureKind.Invalid, cause = error) }
    return NativeVectorEraser(handle,this)
}
private class NativeVectorEraser(private val handle: ProjectSession, private val project: WorkbenchProject) : WorkbenchVectorEraser {
    override suspend fun erase(options: VectorEraseEdit): VectorEraseReceipt {
        val owned = options.ownedEraseRequest()
        val documents = project as? WorkbenchDocumentsProject
        val descriptor = documents?.documentDescriptor(owned.binding.documentId)
        val page = documents?.takeIf { descriptor != null }?.selectedDocumentPage(owned.binding.documentId)
        if (owned.page != page) throw WorkflowFailure(WorkflowFailureKind.Stale)
        val request = NOptions(workflowBinding(owned.binding), workflowMetadata(owned.metadata),
            owned.targets.map { NTarget(it.objectId, it.replacementId) }, owned.centers.map { NPoint(it.x, it.y) },
            owned.radius, owned.memoryBudgetBytes, owned.maxOutputVertices, owned.maxWorkUnits,owned.page)
        return settledWorkflow { cancel ->
            val receipt = handle.eraseVectorStrokes(request, cancel)
            VectorEraseReceipt(receipt.transactionId,
                receipt.changed.map { VectorEraseReplacement(it.originalId, it.outlineId, it.contourCount) },
                workflowInfo(receipt.revision), receipt.estimatedPeakBytes)
        }
    }
}
