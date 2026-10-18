package com.visualworkbench.shared

public data class ImageToneSettings(
    public val brightness: Double = 0.0,
    public val contrast: Double = 0.0,
    public val inputBlack: Double = 0.0,
    public val inputWhite: Double = 1.0,
    public val gamma: Double = 1.0,
    public val outputBlack: Double = 0.0,
    public val outputWhite: Double = 1.0,
)

public data class ToneControlEntry(
    public val layerId: String,
    public val objectId: String,
    public val enabled: Boolean,
    public val locked: Boolean,
    public val opacity: Double,
    public val settings: ImageToneSettings,
)

public data class ToneControlSnapshot(
    public val binding: WorkflowBinding,
    public val sourceAssetId: String,
    public val sourceWidth: UInt,
    public val sourceHeight: UInt,
    public val sourceBitDepth: UByte,
    public val tones: List<ToneControlEntry>,
)

public sealed interface ToneControlCommand {
    public data class Set(public val layerId: String, public val objectId: String,
        public val settings: ImageToneSettings, public val enabled: Boolean, public val opacity: Double) : ToneControlCommand
    public data class Remove(public val layerId: String, public val objectId: String) : ToneControlCommand
}

/** One exact source-space display region, at most 512 by 512 pixels. RGBA8 sRGB
 * is an explicit preview conversion, never a replacement for original pixels.
 * Admit only against its source asset and full visible revision fence. */
public data class ToneControlPreview(
    public val binding: WorkflowBinding,
    public val sourceAssetId: String,
    public val x: UInt,
    public val y: UInt,
    public val width: UInt,
    public val height: UInt,
    public val rgba: ByteArray,
) {
    override fun toString(): String = "ToneControlPreview(binding=$binding, sourceAssetId=$sourceAssetId, " +
        "x=$x, y=$y, width=$width, height=$height, rgbaBytes=${rgba.size})"
}

/** Injected borrowed capability. It owns no project, manager or engine. Snapshot
 * and preview are read-only; only committing an explicitly prepared immutable
 * workflow plan changes canonical parameters. A preview command overlays a
 * private draft without writing an object, transaction or blob. */
public interface WorkbenchToneControls {
    public suspend fun snapshot(documentId: String,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): ToneControlSnapshot
    public suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: ToneControlCommand,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchWorkflowPlan
    public suspend fun preview(binding: WorkflowBinding, command: ToneControlCommand?, region: SelectionRegion,
        assumeUntaggedSrgb: Boolean, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): ToneControlPreview
}

internal fun ToneControlCommand.validateToneControl() {
    val layer = when (this) { is ToneControlCommand.Set -> layerId; is ToneControlCommand.Remove -> layerId }
    val obj = when (this) { is ToneControlCommand.Set -> objectId; is ToneControlCommand.Remove -> objectId }
    if (layer.length != 36 || obj.length != 36 || layer == obj) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    if (this is ToneControlCommand.Set) settings.validateToneSettings(opacity)
}

internal fun ImageToneSettings.validateToneSettings(opacity: Double) {
    if (!brightness.isFinite() || brightness !in -1.0..1.0 || !contrast.isFinite() || contrast !in -1.0..1.0 ||
        listOf(inputBlack,inputWhite,outputBlack,outputWhite).any { !it.isFinite() || it !in 0.0..1.0 } ||
        inputBlack >= inputWhite || outputBlack > outputWhite || !gamma.isFinite() || gamma <= 0.0 ||
        !opacity.isFinite() || opacity !in 0.0..1.0) throw WorkflowFailure(WorkflowFailureKind.Invalid)
}

/** Refuse incomplete/conflicting catalogs rather than silently selecting a
 * truncated target. Copy injected caller-owned collections before publication. */
internal fun ToneControlSnapshot.ownedToneControlSnapshot(): ToneControlSnapshot {
    if (sourceWidth == 0u || sourceHeight == 0u || sourceBitDepth.toInt() !in listOf(8,16) ||
        sourceAssetId.length != 64 || sourceAssetId.any { it !in '0'..'9' && it !in 'a'..'f' } ||
        tones.size > 256 || tones.map { it.objectId }.distinct().size != tones.size ||
        tones.map { it.layerId }.distinct().size != tones.size) throw WorkflowFailure(WorkflowFailureKind.Storage)
    tones.forEach { ToneControlCommand.Set(it.layerId,it.objectId,it.settings,it.enabled,it.opacity).validateToneControl() }
    return copy(tones = tones.toList())
}

internal fun ToneControlSnapshot.previewRegion(x: UInt, y: UInt): SelectionRegion {
    if (x >= sourceWidth || y >= sourceHeight) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return SelectionRegion(x,y,minOf(512u,sourceWidth-x),minOf(512u,sourceHeight-y))
}

internal fun ToneControlSnapshot.validateToneRegion(region: SelectionRegion) {
    if (region.width !in 1u..512u || region.height !in 1u..512u ||
        region.x.toULong()+region.width.toULong() > sourceWidth.toULong() ||
        region.y.toULong()+region.height.toULong() > sourceHeight.toULong()) throw WorkflowFailure(WorkflowFailureKind.Invalid)
}

internal fun ToneControlPreview.ownedToneControlPreview(snapshot: ToneControlSnapshot, region: SelectionRegion): ToneControlPreview {
    snapshot.validateToneRegion(region)
    if (binding != snapshot.binding || sourceAssetId != snapshot.sourceAssetId || x != region.x || y != region.y ||
        width != region.width || height != region.height ||
        rgba.size.toULong() != width.toULong()*height.toULong()*4uL) throw WorkflowFailure(WorkflowFailureKind.Stale)
    return copy(rgba = rgba.copyOf())
}
