package com.visualworkbench.shared

/** Immutable source D pixels remain intact. A frame defines the separate output
 * O grid: integral D crop, clockwise quarter turn, then independent-axis resize.
 * Pixel sampling, tone evaluation and color conversion remain in the native engine. */
public data class ImageFrameSettings(
    public val cropX: UInt, public val cropY: UInt,
    public val cropWidth: UInt, public val cropHeight: UInt,
    public val clockwiseDegrees: UShort,
    public val outputWidth: UInt, public val outputHeight: UInt,
)

public data class ImageFrameEntry(
    public val layerId: String, public val objectId: String,
    public val enabled: Boolean, public val locked: Boolean,
    public val settings: ImageFrameSettings,
)

public data class ImageFramesSnapshot(
    public val binding: WorkflowBinding, public val sourceAssetId: String,
    public val sourceWidth: UInt, public val sourceHeight: UInt,
    public val sourceBitDepth: UByte, public val frame: ImageFrameEntry?,
)

public sealed interface ImageFrameCommand {
    public data class Set(public val layerId: String, public val objectId: String,
        public val settings: ImageFrameSettings, public val enabled: Boolean) : ImageFrameCommand
    public data class Remove(public val layerId: String, public val objectId: String) : ImageFrameCommand
}

/** One exact O-grid display region, at most 512 by 512 pixels, evaluated with
 * the requested saved or private frame. RGBA8 sRGB is a display conversion,
 * never original or export data. A marked preview already contains canonical
 * annotations; callers must not double-paint those annotations over this tile. */
public data class ImageFramePreview(
    public val binding: WorkflowBinding, public val sourceAssetId: String,
    public val frame: ImageFrameSettings?, public val outputWidth: UInt, public val outputHeight: UInt,
    public val x: UInt, public val y: UInt, public val width: UInt, public val height: UInt,
    public val rgba: ByteArray,
) {
    override fun toString(): String = "ImageFramePreview(binding=$binding, sourceAssetId=$sourceAssetId, " +
        "frame=$frame, outputWidth=$outputWidth, outputHeight=$outputHeight, " +
        "x=$x, y=$y, width=$width, height=$height, rgbaBytes=${rgba.size})"
}

/** One exact source D-grid display region, at most 512 by 512 pixels. It contains
 * the canonical editing composition, including canonical annotations and tones. Callers
 * must not double-paint canonical annotations over it. A saved output crop does
 * not hide D pixels here. RGBA8 sRGB is a display conversion, never source data. */
public data class ImageEditingPreview(
    public val binding: WorkflowBinding, public val sourceAssetId: String,
    public val x: UInt, public val y: UInt, public val width: UInt, public val height: UInt,
    public val rgba: ByteArray,
) {
    override fun toString(): String = "ImageEditingPreview(binding=$binding, sourceAssetId=$sourceAssetId, " +
        "x=$x, y=$y, width=$width, height=$height, rgbaBytes=${rgba.size})"
}

/** Whole D-space display background, bounded to 50 million pixels and the
 * caller's budget. It contains canonical annotations and tones, without an output
 * crop. The caller must admit its exact binding/source/dimensions before replacing
 * the background and suppressing canonical vectors, so annotations are not painted
 * twice. RGBA8 sRGB is a display conversion; sourceBitDepth describes the original.
 * The ICC bytes describe the display pixels, never a replacement source profile. */
public data class ImageEditingBackground(
    public val binding: WorkflowBinding, public val sourceAssetId: String, public val sourceBitDepth: UByte,
    public val width: UInt, public val height: UInt, public val rgba: ByteArray, public val iccProfile: ByteArray,
) {
    override fun toString(): String = "ImageEditingBackground(binding=$binding, sourceAssetId=$sourceAssetId, " +
        "sourceBitDepth=$sourceBitDepth, width=$width, height=$height, " +
        "rgbaBytes=${rgba.size}, iccProfileBytes=${iccProfile.size})"
}

/** Borrowed capability over the existing project. Read operations write no
 * object, transaction or blob. Only committing an explicitly prepared immutable
 * workflow plan changes the canonical frame; tones and document owners are reused. */
public interface WorkbenchImageFrames {
    public suspend fun snapshot(documentId: String,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): ImageFramesSnapshot
    public suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: ImageFrameCommand,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchWorkflowPlan
    public suspend fun preview(binding: WorkflowBinding, command: ImageFrameCommand?, region: SelectionRegion,
        marked: Boolean, assumeUntaggedSrgb: Boolean,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): ImageFramePreview
    public suspend fun editingView(binding: WorkflowBinding, region: SelectionRegion, assumeUntaggedSrgb: Boolean,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): ImageEditingPreview
    public suspend fun editingBackground(binding: WorkflowBinding, assumeUntaggedSrgb: Boolean,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): ImageEditingBackground
}

public val ImageFramesSnapshot.outputWidth: UInt get() = frame?.takeIf { it.enabled }?.settings?.outputWidth ?: sourceWidth
public val ImageFramesSnapshot.outputHeight: UInt get() = frame?.takeIf { it.enabled }?.settings?.outputHeight ?: sourceHeight

internal fun ImageFrameSettings.validateImageFrame() {
    if (cropWidth == 0u || cropHeight == 0u || outputWidth !in 1u..32768u || outputHeight !in 1u..32768u ||
        outputWidth.toULong()*outputHeight.toULong() > 50_000_000uL ||
        clockwiseDegrees.toInt() !in listOf(0,90,180,270)) throw WorkflowFailure(WorkflowFailureKind.Invalid)
}

internal fun ImageFrameSettings.validateImageFrameSource(snapshot: ImageFramesSnapshot) {
    validateImageFrame()
    if (snapshot.sourceWidth > 32768u || snapshot.sourceHeight > 32768u ||
        cropX.toULong()+cropWidth.toULong() > snapshot.sourceWidth.toULong() ||
        cropY.toULong()+cropHeight.toULong() > snapshot.sourceHeight.toULong()) throw WorkflowFailure(WorkflowFailureKind.Invalid)
}

internal fun ImageFrameCommand.validateImageFrameCommand() {
    val layer = when (this) { is ImageFrameCommand.Set -> layerId; is ImageFrameCommand.Remove -> layerId }
    val obj = when (this) { is ImageFrameCommand.Set -> objectId; is ImageFrameCommand.Remove -> objectId }
    if (layer.length != 36 || obj.length != 36 || layer == obj) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    if (this is ImageFrameCommand.Set) settings.validateImageFrame()
}

internal fun ImageFramesSnapshot.ownedImageFramesSnapshot(): ImageFramesSnapshot {
    if (sourceWidth == 0u || sourceHeight == 0u || sourceBitDepth.toInt() !in listOf(8,16) ||
        sourceAssetId.length != 64 || sourceAssetId.any { it !in '0'..'9' && it !in 'a'..'f' })
        throw WorkflowFailure(WorkflowFailureKind.Storage)
    frame?.let {
        ImageFrameCommand.Set(it.layerId,it.objectId,it.settings,it.enabled).validateImageFrameCommand()
        it.settings.validateImageFrameSource(this)
    }
    return copy()
}

internal fun ImageFramesSnapshot.effectiveFrame(command: ImageFrameCommand?): ImageFrameSettings? = when (command) {
    null -> frame?.takeIf { it.enabled }?.settings
    is ImageFrameCommand.Set -> command.settings.takeIf { command.enabled }
    is ImageFrameCommand.Remove -> null
}

internal fun imageFrameRegion(x: UInt, y: UInt, width: UInt, height: UInt): SelectionRegion {
    if (x >= width || y >= height) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return SelectionRegion(x,y,minOf(512u,width-x),minOf(512u,height-y))
}

internal fun validateImageFrameRegion(region: SelectionRegion, width: UInt, height: UInt) {
    if (region.width !in 1u..512u || region.height !in 1u..512u ||
        region.x.toULong()+region.width.toULong() > width.toULong() ||
        region.y.toULong()+region.height.toULong() > height.toULong()) throw WorkflowFailure(WorkflowFailureKind.Invalid)
}

internal fun ImageFramePreview.ownedImageFramePreview(snapshot: ImageFramesSnapshot, command: ImageFrameCommand?,
    region: SelectionRegion): ImageFramePreview {
    val expected = snapshot.effectiveFrame(command)
    val expectedWidth = expected?.outputWidth ?: snapshot.sourceWidth
    val expectedHeight = expected?.outputHeight ?: snapshot.sourceHeight
    validateImageFrameRegion(region,expectedWidth,expectedHeight)
    if (binding != snapshot.binding || sourceAssetId != snapshot.sourceAssetId || frame != expected ||
        outputWidth != expectedWidth || outputHeight != expectedHeight || x != region.x || y != region.y ||
        width != region.width || height != region.height || rgba.size.toULong() != width.toULong()*height.toULong()*4uL)
        throw WorkflowFailure(WorkflowFailureKind.Stale)
    return copy(rgba=rgba.copyOf())
}

internal fun ImageEditingPreview.ownedImageEditingPreview(snapshot: ImageFramesSnapshot,
    region: SelectionRegion): ImageEditingPreview {
    validateImageFrameRegion(region,snapshot.sourceWidth,snapshot.sourceHeight)
    if (binding != snapshot.binding || sourceAssetId != snapshot.sourceAssetId || x != region.x || y != region.y ||
        width != region.width || height != region.height || rgba.size.toULong() != width.toULong()*height.toULong()*4uL)
        throw WorkflowFailure(WorkflowFailureKind.Stale)
    return copy(rgba=rgba.copyOf())
}

internal fun ImageEditingBackground.ownedImageEditingBackground(expected: WorkflowBinding,
    memoryBudgetBytes: ULong): ImageEditingBackground {
    val pixels = width.toULong()*height.toULong()
    if (pixels > 50_000_000uL) throw WorkflowFailure(WorkflowFailureKind.Limit)
    if (binding != expected || width == 0u || height == 0u || sourceBitDepth.toInt() !in listOf(8,16) ||
        sourceAssetId.length != 64 || sourceAssetId.any { it !in '0'..'9' && it !in 'a'..'f' } ||
        rgba.size.toULong() != pixels*4uL) throw WorkflowFailure(WorkflowFailureKind.Stale)
    if (rgba.size.toULong()+iccProfile.size.toULong() > memoryBudgetBytes)
        throw WorkflowFailure(WorkflowFailureKind.Memory)
    return copy(rgba=rgba.copyOf(),iccProfile=iccProfile.copyOf())
}
