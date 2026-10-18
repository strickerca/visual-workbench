package com.visualworkbench.shared

/** Private bounded numeric text. Intermediate typing is retained; an explicit
 * preview or Apply validates the complete form. Width and height are independent
 * choices: rotation never silently swaps or corrects them or locks an aspect. */
public data class ImageFramesForm(
    public val cropX: String, public val cropY: String,
    public val cropWidth: String, public val cropHeight: String,
    public val clockwiseDegrees: UShort,
    public val outputWidth: String, public val outputHeight: String,
    public val enabled: Boolean = true,
)

internal fun ImageFramesForm.bounded(): Boolean =
    listOf(cropX,cropY,cropWidth,cropHeight,outputWidth,outputHeight).all { it.length <= 32 } &&
        clockwiseDegrees.toInt() in listOf(0,90,180,270)

internal fun ImageFramesSnapshot.form(): ImageFramesForm {
    val value = frame?.settings ?: ImageFrameSettings(0u,0u,sourceWidth,sourceHeight,0u,sourceWidth,sourceHeight)
    return ImageFramesForm(value.cropX.toString(),value.cropY.toString(),value.cropWidth.toString(),value.cropHeight.toString(),
        value.clockwiseDegrees,value.outputWidth.toString(),value.outputHeight.toString(),frame?.enabled ?: true)
}

internal fun ImageFramesForm.command(snapshot: ImageFramesSnapshot,
    newIds: () -> Pair<String,String>): ImageFrameCommand.Set {
    // Validate every value and source bound before consuming any identities.
    val value = settings(snapshot)
    val ids = snapshot.frame?.let { it.layerId to it.objectId } ?: newIds()
    return ImageFrameCommand.Set(ids.first,ids.second,value,enabled).also { it.validateImageFrameCommand() }
}

internal fun ImageFramesForm.settings(snapshot: ImageFramesSnapshot): ImageFrameSettings {
    fun integer(text: String): UInt = text.takeIf { it.isNotEmpty() && it.all { c -> c in '0'..'9' } }
        ?.toUIntOrNull() ?: throw WorkflowFailure(WorkflowFailureKind.Invalid)
    if (!bounded()) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    if (snapshot.frame?.locked == true) throw WorkflowFailure(WorkflowFailureKind.Locked)
    return ImageFrameSettings(integer(cropX),integer(cropY),integer(cropWidth),integer(cropHeight),
        clockwiseDegrees,integer(outputWidth),integer(outputHeight)).also { it.validateImageFrameSource(snapshot) }
}
