package com.visualworkbench.shared

/** Original D pixels are immutable. These settings define the separate O export
 * grid: integral D crop, clockwise quarter turn, then independent-axis resize.
 * The native resampler uses premultiplied bilinear source-space samples. */
public data class ImageOutputSettings(public val cropX: UInt, public val cropY: UInt,
    public val cropWidth: UInt, public val cropHeight: UInt, public val clockwiseDegrees: UShort,
    public val outputWidth: UInt, public val outputHeight: UInt)
public data class ImageToneSettings(public val brightness: Double = 0.0, public val contrast: Double = 0.0,
    public val inputBlack: Double = 0.0, public val inputWhite: Double = 1.0, public val gamma: Double = 1.0,
    public val outputBlack: Double = 0.0, public val outputWhite: Double = 1.0)
public data class ImageOutputEntry(public val layerId: String, public val objectId: String,
    public val enabled: Boolean, public val locked: Boolean, public val settings: ImageOutputSettings)
public data class ImageToneEntry(public val layerId: String, public val objectId: String,
    public val enabled: Boolean, public val opacity: Double, public val locked: Boolean, public val settings: ImageToneSettings)
public data class ImageOutputSnapshot(public val binding: WorkflowBinding, public val sourceAssetId: String,
    public val sourceWidth: UInt, public val sourceHeight: UInt, public val sourceBitDepth: UByte,
    public val output: ImageOutputEntry?, public val tones: List<ImageToneEntry>)
public sealed interface ImageOutputCommand {
    public data class SetFrame(public val layerId: String, public val objectId: String,
        public val settings: ImageOutputSettings, public val enabled: Boolean) : ImageOutputCommand
    public data class RemoveFrame(public val layerId: String, public val objectId: String) : ImageOutputCommand
    public data class SetTone(public val layerId: String, public val objectId: String,
        public val settings: ImageToneSettings, public val enabled: Boolean, public val opacity: Double) : ImageOutputCommand
    public data class RemoveTone(public val layerId: String, public val objectId: String) : ImageOutputCommand
}
/** Exact O-grid region, up to 512×512. sRGB RGBA8 is an explicit display conversion,
 * never original or export data. The returned source/revision/frame must match
 * the current view before an app paints it. */
public data class ImageOutputPreview(public val binding: WorkflowBinding, public val sourceAssetId: String,
    public val output: ImageOutputSettings?, public val outputWidth: UInt, public val outputHeight: UInt,
    public val x: UInt, public val y: UInt, public val width: UInt, public val height: UInt, public val rgba: ByteArray)
public interface WorkbenchImageOutputs {
    public suspend fun snapshot(documentId: String, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): ImageOutputSnapshot
    public suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: ImageOutputCommand,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchWorkflowPlan
    /** Paints the committed settings. Private form edits are committed only by an
     * explicit immutable plan; this query performs no edit and publishes no blob. */
    public suspend fun preview(binding: WorkflowBinding, region: SelectionRegion, marked: Boolean,
        assumeUntaggedSrgb: Boolean, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): ImageOutputPreview
}
public expect fun imageOutputWorkflows(project: WorkbenchProject): WorkbenchImageOutputs
public fun WorkbenchProject.imageOutputs(): WorkbenchImageOutputs = imageOutputWorkflows(this)

internal fun ImageOutputCommand.validateImageOutput() {
    fun ids(layer: String, obj: String) { if (layer.length != 36 || obj.length != 36 || layer == obj) throw WorkflowFailure(WorkflowFailureKind.Invalid) }
    fun frame(value: ImageOutputSettings) {
        if (value.cropWidth == 0u || value.cropHeight == 0u || value.outputWidth == 0u || value.outputHeight == 0u ||
            value.clockwiseDegrees.toInt() !in listOf(0, 90, 180, 270) || value.outputWidth > 32768u || value.outputHeight > 32768u ||
            value.outputWidth.toULong() * value.outputHeight.toULong() > 50_000_000uL) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    }
    fun tone(value: ImageToneSettings, opacity: Double) {
        val levels = listOf(value.inputBlack, value.inputWhite, value.outputBlack, value.outputWhite)
        if (!value.brightness.isFinite() || value.brightness !in -1.0..1.0 || !value.contrast.isFinite() || value.contrast !in -1.0..1.0 ||
            levels.any { !it.isFinite() || it !in 0.0..1.0 } || value.inputBlack >= value.inputWhite || value.outputBlack > value.outputWhite ||
            !value.gamma.isFinite() || value.gamma <= 0.0 || !opacity.isFinite() || opacity !in 0.0..1.0) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    }
    when (this) {
        is ImageOutputCommand.SetFrame -> { ids(layerId, objectId); frame(settings) }
        is ImageOutputCommand.RemoveFrame -> ids(layerId, objectId)
        is ImageOutputCommand.SetTone -> { ids(layerId, objectId); tone(settings, opacity) }
        is ImageOutputCommand.RemoveTone -> ids(layerId, objectId)
    }
}
