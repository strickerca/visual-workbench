package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.ImageOutputCommand as NCommand
import com.visualworkbench.bindings.core.ImageOutputSettings as NSettings
import com.visualworkbench.bindings.core.ImageToneSettings as NTone
import com.visualworkbench.bindings.core.SelectionRegion as NRegion

public actual fun imageOutputWorkflows(project: WorkbenchProject): WorkbenchImageOutputs = NativeImageOutputs(nativeProjectHandle(project))
private fun ImageOutputSettings.native(): NSettings = NSettings(cropX,cropY,cropWidth,cropHeight,clockwiseDegrees,outputWidth,outputHeight)
private fun NSettings.common(): ImageOutputSettings = ImageOutputSettings(cropX,cropY,cropWidth,cropHeight,clockwiseDegrees,outputWidth,outputHeight)
private fun ImageToneSettings.native(): NTone = NTone(brightness,contrast,inputBlack,inputWhite,gamma,outputBlack,outputWhite)
private fun NTone.common(): ImageToneSettings = ImageToneSettings(brightness,contrast,inputBlack,inputWhite,gamma,outputBlack,outputWhite)
private fun ImageOutputCommand.native(): NCommand = when (this) {
    is ImageOutputCommand.SetFrame -> NCommand.SetFrame(layerId,objectId,settings.native(),enabled)
    is ImageOutputCommand.RemoveFrame -> NCommand.RemoveFrame(layerId,objectId)
    is ImageOutputCommand.SetTone -> NCommand.SetTone(layerId,objectId,settings.native(),enabled,opacity)
    is ImageOutputCommand.RemoveTone -> NCommand.RemoveTone(layerId,objectId)
}
private class NativeImageOutputs(private val handle: ProjectSession) : WorkbenchImageOutputs {
    override suspend fun snapshot(documentId: String, memoryBudgetBytes: ULong): ImageOutputSnapshot {
        if (documentId.length != 36) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        return settledWorkflow { cancellation ->
            val value = handle.imageOutputSnapshot(documentId,memoryBudgetBytes,cancellation)
            ImageOutputSnapshot(workflowBinding(value.binding),value.sourceAssetId,value.sourceWidth,value.sourceHeight,value.sourceBitDepth,
                value.output?.let { ImageOutputEntry(it.layerId,it.objectId,it.enabled,it.locked,it.settings.common()) },
                value.tones.map { ImageToneEntry(it.layerId,it.objectId,it.enabled,it.opacity,it.locked,it.settings.common()) })
        }
    }
    override suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: ImageOutputCommand,
        memoryBudgetBytes: ULong): WorkbenchWorkflowPlan {
        command.validateImageOutput()
        val expected = workflowBinding(binding)
        val author = workflowMetadata(metadata)
        val owned = command.native()
        val plan = settledWorkflow(release = ::closeWorkflowPlan) { cancellation ->
            handle.prepareImageOutput(expected,author,owned,memoryBudgetBytes,cancellation)
        }
        return wrapWorkflowPlan(plan)
    }
    override suspend fun preview(binding: WorkflowBinding, region: SelectionRegion, marked: Boolean,
        assumeUntaggedSrgb: Boolean, memoryBudgetBytes: ULong): ImageOutputPreview {
        if (region.width !in 1u..512u || region.height !in 1u..512u) throw WorkflowFailure(WorkflowFailureKind.Limit)
        val expected = workflowBinding(binding)
        val requested = NRegion(region.x,region.y,region.width,region.height)
        return settledWorkflow { cancellation ->
            val value = handle.imageOutputPreview(expected,requested,marked,assumeUntaggedSrgb,memoryBudgetBytes,cancellation)
            ImageOutputPreview(workflowBinding(value.binding),value.sourceAssetId,value.output?.common(),value.outputWidth,value.outputHeight,
                value.x,value.y,value.width,value.height,value.rgba)
        }
    }
}
