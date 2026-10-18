package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.ImageFrameSettings as NSettings
import com.visualworkbench.bindings.core.ImageFrameCommand as NCommand
import com.visualworkbench.bindings.core.SelectionRegion as NRegion

/** Borrow image-frame workflows from the existing project session. */
public fun imageFrameWorkflows(project: WorkbenchProject): WorkbenchImageFrames =
    NativeImageFrames(nativeProjectHandle(project))

private fun ImageFrameSettings.native(): NSettings =
    NSettings(cropX,cropY,cropWidth,cropHeight,clockwiseDegrees,outputWidth,outputHeight)

private fun NSettings.common(): ImageFrameSettings =
    ImageFrameSettings(cropX,cropY,cropWidth,cropHeight,clockwiseDegrees,outputWidth,outputHeight)

private fun ImageFrameCommand.native(): NCommand = when (this) {
    is ImageFrameCommand.Set -> NCommand.Set(layerId,objectId,settings.native(),enabled)
    is ImageFrameCommand.Remove -> NCommand.Remove(layerId,objectId)
}

private class NativeImageFrames(private val handle: ProjectSession) : WorkbenchImageFrames {
    override suspend fun snapshot(documentId: String, memoryBudgetBytes: ULong): ImageFramesSnapshot {
        if (documentId.length != 36) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        return settledWorkflow { cancellation ->
            val value = handle.imageFramesSnapshot(documentId,memoryBudgetBytes,cancellation)
            ImageFramesSnapshot(workflowBinding(value.binding),value.sourceAssetId,value.sourceWidth,
                value.sourceHeight,value.sourceBitDepth,value.frame?.let {
                    ImageFrameEntry(it.layerId,it.objectId,it.enabled,it.locked,it.settings.common())
                }).ownedImageFramesSnapshot()
        }
    }

    override suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: ImageFrameCommand,
        memoryBudgetBytes: ULong): WorkbenchWorkflowPlan {
        command.validateImageFrameCommand()
        val expected = workflowBinding(binding)
        val author = workflowMetadata(metadata)
        val owned = command.native()
        val plan = settledWorkflow(release=::closeWorkflowPlan) { cancellation ->
            handle.prepareImageFrame(expected,author,owned,memoryBudgetBytes,cancellation)
        }
        return wrapWorkflowPlan(plan)
    }

    override suspend fun preview(binding: WorkflowBinding, command: ImageFrameCommand?, region: SelectionRegion,
        marked: Boolean, assumeUntaggedSrgb: Boolean, memoryBudgetBytes: ULong): ImageFramePreview {
        command?.validateImageFrameCommand()
        validateTile(region)
        val expected = workflowBinding(binding)
        val draft = command?.native()
        val tile = NRegion(region.x,region.y,region.width,region.height)
        return settledWorkflow { cancellation ->
            val value = handle.imageFramePreview(expected,draft,tile,marked,assumeUntaggedSrgb,memoryBudgetBytes,cancellation)
            val common = ImageFramePreview(workflowBinding(value.binding),value.sourceAssetId,value.frame?.common(),
                value.outputWidth,value.outputHeight,value.x,value.y,value.width,value.height,value.rgba)
            validateReply(common.binding,binding,common.sourceAssetId,region,common.x,common.y,common.width,common.height,common.rgba)
            validateImageFrameRegion(region,common.outputWidth,common.outputHeight)
            common.frame?.let {
                it.validateImageFrame()
                if (common.outputWidth != it.outputWidth || common.outputHeight != it.outputHeight)
                    throw WorkflowFailure(WorkflowFailureKind.Stale)
            }
            if (command != null) {
                val requested = (command as? ImageFrameCommand.Set)?.takeIf { it.enabled }?.settings
                if (common.frame != requested) throw WorkflowFailure(WorkflowFailureKind.Stale)
            }
            common.copy(rgba=common.rgba.copyOf())
        }
    }

    override suspend fun editingView(binding: WorkflowBinding, region: SelectionRegion, assumeUntaggedSrgb: Boolean,
        memoryBudgetBytes: ULong): ImageEditingPreview {
        validateTile(region)
        val expected = workflowBinding(binding)
        val tile = NRegion(region.x,region.y,region.width,region.height)
        return settledWorkflow { cancellation ->
            val value = handle.imageEditingPreview(expected,tile,assumeUntaggedSrgb,memoryBudgetBytes,cancellation)
            val common = ImageEditingPreview(workflowBinding(value.binding),value.sourceAssetId,value.x,value.y,
                value.width,value.height,value.rgba)
            validateReply(common.binding,binding,common.sourceAssetId,region,common.x,common.y,common.width,common.height,common.rgba)
            common.copy(rgba=common.rgba.copyOf())
        }
    }

    override suspend fun editingBackground(binding: WorkflowBinding, assumeUntaggedSrgb: Boolean,
        memoryBudgetBytes: ULong): ImageEditingBackground {
        val expected = workflowBinding(binding)
        return settledWorkflow { cancellation ->
            val value = handle.imageEditingBackground(expected,assumeUntaggedSrgb,memoryBudgetBytes,cancellation)
            ImageEditingBackground(workflowBinding(value.binding),value.sourceAssetId,value.sourceBitDepth,
                value.width,value.height,value.rgba,value.iccProfile).ownedImageEditingBackground(binding,memoryBudgetBytes)
        }
    }

}

private fun validateTile(region: SelectionRegion) {
    if (region.width !in 1u..512u || region.height !in 1u..512u) throw WorkflowFailure(WorkflowFailureKind.Invalid)
}

private fun validateReply(actual: WorkflowBinding, expected: WorkflowBinding, source: String, region: SelectionRegion,
    x: UInt, y: UInt, width: UInt, height: UInt, rgba: ByteArray) {
    if (actual != expected || source.length != 64 || source.any { it !in '0'..'9' && it !in 'a'..'f' } ||
        x != region.x || y != region.y || width != region.width || height != region.height ||
        rgba.size.toULong() != width.toULong()*height.toULong()*4uL) throw WorkflowFailure(WorkflowFailureKind.Stale)
}
