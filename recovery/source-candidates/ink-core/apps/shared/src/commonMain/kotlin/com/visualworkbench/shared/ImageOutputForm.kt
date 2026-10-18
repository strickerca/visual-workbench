package com.visualworkbench.shared

/** Bounded private text, never a canonical object or export option. Invalid
 * intermediate typing is retained until explicit Apply validates the whole form. */
public sealed interface ImageOutputForm {
    public data class Frame(
        public val cropX: String, public val cropY: String,
        public val cropWidth: String, public val cropHeight: String,
        public val clockwiseDegrees: UShort,
        public val outputWidth: String, public val outputHeight: String,
        public val enabled: Boolean = true,
    ) : ImageOutputForm
    public data class Tone(
        public val objectId: String?,
        public val brightness: String = "0.0", public val contrast: String = "0.0",
        public val inputBlack: String = "0.0", public val inputWhite: String = "1.0",
        public val gamma: String = "1.0", public val outputBlack: String = "0.0",
        public val outputWhite: String = "1.0", public val opacity: String = "1.0",
        public val enabled: Boolean = true,
    ) : ImageOutputForm
}

internal fun ImageOutputForm.bounded(): Boolean = when (this) {
    is ImageOutputForm.Frame -> listOf(cropX,cropY,cropWidth,cropHeight,outputWidth,outputHeight)
        .all { it.length <= 32 } && clockwiseDegrees.toInt() in listOf(0,90,180,270)
    is ImageOutputForm.Tone -> listOf(brightness,contrast,inputBlack,inputWhite,gamma,outputBlack,outputWhite,opacity)
        .all { it.length <= 32 } && (objectId == null || objectId.length == 36)
}

internal fun ImageOutputSnapshot.form(selectedTone: String? = null, tone: Boolean = false): ImageOutputForm {
    if (tone) {
        val entry = tones.firstOrNull { it.objectId == selectedTone }
        val settings = entry?.settings ?: ImageToneSettings()
        return ImageOutputForm.Tone(entry?.objectId,settings.brightness.toString(),settings.contrast.toString(),
            settings.inputBlack.toString(),settings.inputWhite.toString(),settings.gamma.toString(),
            settings.outputBlack.toString(),settings.outputWhite.toString(),(entry?.opacity ?: 1.0).toString(),entry?.enabled ?: true)
    }
    val settings = output?.settings ?: ImageOutputSettings(0u,0u,sourceWidth,sourceHeight,0u,sourceWidth,sourceHeight)
    return ImageOutputForm.Frame(settings.cropX.toString(),settings.cropY.toString(),settings.cropWidth.toString(),
        settings.cropHeight.toString(),settings.clockwiseDegrees,settings.outputWidth.toString(),settings.outputHeight.toString(),
        output?.enabled ?: true)
}

internal fun ImageOutputForm.command(snapshot: ImageOutputSnapshot, newId: () -> String): ImageOutputCommand {
    fun invalid(): Nothing = throw WorkflowFailure(WorkflowFailureKind.Invalid)
    fun integer(text: String): UInt = text.takeIf { it.isNotEmpty() && it.all { c -> c in '0'..'9' } }?.toUIntOrNull() ?: invalid()
    fun number(text: String): Double = text.toDoubleOrNull()?.takeIf { it.isFinite() } ?: invalid()
    if (!bounded()) invalid()
    return when (this) {
        is ImageOutputForm.Frame -> {
            val value = ImageOutputSettings(integer(cropX),integer(cropY),integer(cropWidth),integer(cropHeight),
                clockwiseDegrees,integer(outputWidth),integer(outputHeight))
            if (value.cropX.toULong()+value.cropWidth.toULong()>snapshot.sourceWidth.toULong() ||
                value.cropY.toULong()+value.cropHeight.toULong()>snapshot.sourceHeight.toULong()) invalid()
            if (snapshot.output?.locked == true) throw WorkflowFailure(WorkflowFailureKind.Locked)
            // Validate all text/ranges before acquiring new object identities.
            ImageOutputCommand.SetFrame(VALIDATION_LAYER,VALIDATION_OBJECT,value,enabled).validateImageOutput()
            ImageOutputCommand.SetFrame(snapshot.output?.layerId ?: newId(),snapshot.output?.objectId ?: newId(),value,enabled)
        }
        is ImageOutputForm.Tone -> {
            val old = objectId?.let { id -> snapshot.tones.firstOrNull { it.objectId == id } ?: invalid() }
            if (old?.locked == true) throw WorkflowFailure(WorkflowFailureKind.Locked)
            val value = ImageToneSettings(number(brightness),number(contrast),number(inputBlack),number(inputWhite),
                number(gamma),number(outputBlack),number(outputWhite))
            val alpha = number(opacity)
            ImageOutputCommand.SetTone(VALIDATION_LAYER,VALIDATION_OBJECT,value,enabled,alpha).validateImageOutput()
            ImageOutputCommand.SetTone(old?.layerId ?: newId(),old?.objectId ?: newId(),value,enabled,alpha)
        }
    }.also { it.validateImageOutput() }
}

private const val VALIDATION_LAYER: String = "00000000-0000-4000-8000-000000000001"
private const val VALIDATION_OBJECT: String = "00000000-0000-4000-8000-000000000002"
