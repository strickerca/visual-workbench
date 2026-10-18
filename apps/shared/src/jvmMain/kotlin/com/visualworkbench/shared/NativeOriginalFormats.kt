package com.visualworkbench.shared

import com.visualworkbench.bindings.core.CoreException
import com.visualworkbench.bindings.core.CreateFileImageProject
import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.createFileRawPreviewProject
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

private val rawPreviewRetirements = GiantImportRetirements()
private fun ownRawPreviewProject(value: ProjectSession): GiantImportOwner<ProjectSession> = GiantImportOwner(value,
    { drainUnadoptedGiants(); try { it.closeSession() } catch (error: CoreException) { throw mapFailure(error) } },
    { it.destroy() }, rawPreviewRetirements)

public actual suspend fun createRawPreviewProject(options: CreateFileProject): WorkbenchProject {
    rawPreviewRetirements.drain()
    val native = CreateFileImageProject(options.path, options.projectId, options.documentId, options.layerId,
        options.deviceId, options.title, options.sourcePath, options.workDirectory, options.nowMs,
        options.memoryBudgetBytes, options.maxEncodedBytes, options.maxScratchBytes)
    return acquireGiantImport({ createFileRawPreviewProject(native, it) }, ::ownRawPreviewProject, ::wrapNativeGiantProject)
}

internal suspend fun originalFormatInfoNative(handle: ProjectSession, binding: WorkflowBinding): OriginalFormatInfo {
    currentCoroutineContext().ensureActive()
    return settledWorkflow { cancel ->
        val value = handle.originalFormatInfo(workflowBinding(binding), cancel)
        OriginalFormatInfo(workflowBinding(value.binding), value.sourceAssetId, value.format,
            value.sourceBitDepth, value.untagged, value.firstTiffPageOnly,
            value.flattenedComposite, value.rawPreview).also { validateOriginalFormatInfo(it, binding) }
    }
}
