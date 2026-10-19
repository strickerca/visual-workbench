package com.visualworkbench.shared

/** Private, bounded text. Invalid intermediate typing remains local until an
 * explicit preview or Apply validates the complete form. */
public data class ToneControlForm(
    public val objectId: String? = null,
    public val brightness: String = "0.0",
    public val contrast: String = "0.0",
    public val inputBlack: String = "0.0",
    public val inputWhite: String = "1.0",
    public val gamma: String = "1.0",
    public val outputBlack: String = "0.0",
    public val outputWhite: String = "1.0",
    public val opacity: String = "1.0",
    public val enabled: Boolean = true,
)

internal fun ToneControlForm.bounded(): Boolean =
    listOf(brightness,contrast,inputBlack,inputWhite,gamma,outputBlack,outputWhite,opacity).all { it.length <= 32 } &&
        (objectId == null || objectId.length == 36)

internal fun ToneControlSnapshot.form(objectId: String? = null): ToneControlForm {
    val entry = tones.firstOrNull { it.objectId == objectId }
    val value = entry?.settings ?: ImageToneSettings()
    return ToneControlForm(entry?.objectId,value.brightness.toString(),value.contrast.toString(),value.inputBlack.toString(),
        value.inputWhite.toString(),value.gamma.toString(),value.outputBlack.toString(),value.outputWhite.toString(),
        (entry?.opacity ?: 1.0).toString(),entry?.enabled ?: true)
}

internal fun ToneControlForm.command(snapshot: ToneControlSnapshot, newIds: () -> Pair<String,String>): ToneControlCommand.Set {
    fun invalid(): Nothing = throw WorkflowFailure(WorkflowFailureKind.Invalid)
    fun number(text: String): Double = text.toDoubleOrNull()?.takeIf { it.isFinite() } ?: invalid()
    if (!bounded()) invalid()
    val old = objectId?.let { id -> snapshot.tones.firstOrNull { it.objectId == id } ?: invalid() }
    if (old?.locked == true) throw WorkflowFailure(WorkflowFailureKind.Locked)
    val value = ImageToneSettings(number(brightness),number(contrast),number(inputBlack),number(inputWhite),
        number(gamma),number(outputBlack),number(outputWhite))
    val alpha = number(opacity)
    // Invalid text never consumes identities. The interaction retains a new
    // draft's pair across every preview and its eventual single Apply plan.
    value.validateToneSettings(alpha)
    val ids = old?.let { it.layerId to it.objectId } ?: newIds()
    return ToneControlCommand.Set(ids.first,ids.second,value,enabled,alpha).also { it.validateToneControl() }
}
