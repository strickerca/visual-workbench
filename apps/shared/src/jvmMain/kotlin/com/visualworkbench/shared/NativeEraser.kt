package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.VectorEraseOptions as NOptions
import com.visualworkbench.bindings.core.VectorEraseTarget as NTarget

public actual fun WorkbenchProject.vectorEraser(): WorkbenchVectorEraser {
    val handle = try { nativeProjectHandle(this) }
        catch (error: SessionFailure) { throw WorkflowFailure(WorkflowFailureKind.Invalid, cause = error) }
    return NativeVectorEraser(handle)
}
private class NativeVectorEraser(private val handle: ProjectSession) : WorkbenchVectorEraser {
    override suspend fun erase(options: VectorEraseEdit): VectorEraseReceipt {
        val owned = options.ownedEraseRequest()
        val request = NOptions(workflowBinding(owned.binding), workflowMetadata(owned.metadata),
            owned.targets.map { NTarget(it.objectId, it.replacementId) }, owned.centers.map { NPoint(it.x, it.y) },
            owned.radius, owned.memoryBudgetBytes, owned.maxOutputVertices, owned.maxWorkUnits)
        return settledWorkflow { cancel ->
            val receipt = handle.eraseVectorStrokes(request, cancel)
            VectorEraseReceipt(receipt.transactionId,
                receipt.changed.map { VectorEraseReplacement(it.originalId, it.outlineId, it.contourCount) },
                workflowInfo(receipt.revision), receipt.estimatedPeakBytes)
        }
    }
}
