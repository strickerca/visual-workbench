package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

internal suspend fun originalFormatInfoNative(handle: ProjectSession, binding: WorkflowBinding): OriginalFormatInfo {
    currentCoroutineContext().ensureActive()
    return settledWorkflow { cancel ->
        val value = handle.originalFormatInfo(workflowBinding(binding), cancel)
        OriginalFormatInfo(workflowBinding(value.binding), value.sourceAssetId, value.format,
            value.sourceBitDepth, value.untagged, value.firstTiffPageOnly).also { validateOriginalFormatInfo(it, binding) }
    }
}
