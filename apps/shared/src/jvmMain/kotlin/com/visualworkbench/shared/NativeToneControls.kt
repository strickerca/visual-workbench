package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.ImageToneSettings as NSettings
import com.visualworkbench.bindings.core.SelectionRegion as NRegion
import com.visualworkbench.bindings.core.ToneControlCommand as NCommand

/** Borrow tonal workflows from the existing project session. */
public fun toneControlWorkflows(project: WorkbenchProject): WorkbenchToneControls =
    NativeToneControls(nativeProjectHandle(project))

private fun ImageToneSettings.native(): NSettings =
    NSettings(brightness, contrast, inputBlack, inputWhite, gamma, outputBlack, outputWhite)

private fun NSettings.common(): ImageToneSettings =
    ImageToneSettings(brightness, contrast, inputBlack, inputWhite, gamma, outputBlack, outputWhite)

private fun ToneControlCommand.native(): NCommand = when (this) {
    is ToneControlCommand.Set -> NCommand.Set(layerId, objectId, settings.native(), enabled, opacity)
    is ToneControlCommand.Remove -> NCommand.Remove(layerId, objectId)
}

private class NativeToneControls(private val handle: ProjectSession) : WorkbenchToneControls {
    override suspend fun snapshot(documentId: String, memoryBudgetBytes: ULong): ToneControlSnapshot {
        if (documentId.length != 36) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        return settledWorkflow { cancellation ->
            val value = handle.toneControlsSnapshot(documentId, memoryBudgetBytes, cancellation)
            ToneControlSnapshot(workflowBinding(value.binding), value.sourceAssetId, value.sourceWidth,
                value.sourceHeight, value.sourceBitDepth, value.tones.map {
                    ToneControlEntry(it.layerId, it.objectId, it.enabled, it.locked, it.opacity, it.settings.common())
                }).ownedToneControlSnapshot()
        }
    }

    override suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: ToneControlCommand,
        memoryBudgetBytes: ULong): WorkbenchWorkflowPlan {
        command.validateToneControl()
        val expected = workflowBinding(binding)
        val author = workflowMetadata(metadata)
        val owned = command.native()
        val plan = settledWorkflow(release = ::closeWorkflowPlan) { cancellation ->
            handle.prepareToneControl(expected, author, owned, memoryBudgetBytes, cancellation)
        }
        return wrapWorkflowPlan(plan)
    }

    override suspend fun preview(binding: WorkflowBinding, command: ToneControlCommand?, region: SelectionRegion,
        assumeUntaggedSrgb: Boolean, memoryBudgetBytes: ULong): ToneControlPreview {
        command?.validateToneControl()
        if (region.width !in 1u..512u || region.height !in 1u..512u) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        val expected = workflowBinding(binding)
        val draft = command?.native()
        val tile = NRegion(region.x, region.y, region.width, region.height)
        return settledWorkflow { cancellation ->
            val value = handle.previewToneControl(expected, draft, tile, assumeUntaggedSrgb, memoryBudgetBytes, cancellation)
            if (workflowBinding(value.binding) != binding || value.x != region.x || value.y != region.y ||
                value.width != region.width || value.height != region.height ||
                value.rgba.size.toULong() != region.width.toULong() * region.height.toULong() * 4uL) {
                throw WorkflowFailure(WorkflowFailureKind.Stale)
            }
            ToneControlPreview(workflowBinding(value.binding), value.sourceAssetId, value.x, value.y,
                value.width, value.height, value.rgba.copyOf())
        }
    }
}
