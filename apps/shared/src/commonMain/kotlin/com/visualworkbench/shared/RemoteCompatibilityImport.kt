package com.visualworkbench.shared

/** Only the checked runtime adapter implements this capability in production.
 * Raw file bytes, caller flags, profile digests and input ACKs are not evidence. */
internal interface RemoteCompatibilityImportCapability {
    suspend fun observeCompatibility(expected:RemoteTargetState):RemoteCompatibilityAdmission? = null
    suspend fun admitCompatibilityReceipt(expected:RemoteTargetState,bytes:ByteArray):RemoteCompatibilityAdmission?
}
internal data class RemoteCompatibilityAdmission(
    val target:RemoteTargetState,
    val toolId:String,
    val settingsDigest:String,
    val records:List<RemoteToolCompatibility>,
)
internal const val REMOTE_COMPATIBILITY_IMPORT_LIMIT:Int=32*1024

internal fun RemoteCompatibilityAdmission.validFor(expected:RemoteTargetState):Boolean =
    target.binding==expected.binding&&target.identity==expected.identity&&
        target.identity.editor==RemoteEditorKind.Paint&&target.identity.valid()&&
        toolId.length in 1..128&&toolId.none{it.isISOControl()}&&
        settingsDigest.length==64&&settingsDigest.all{it in '0'..'9'||it in 'a'..'f'}&&
        records.size in 1..RemoteCompatibilityAspect.entries.size&&
        records.map{it.aspect}.distinct().size==records.size&&
        records.all{it.validFor(expected.identity,toolId,settingsDigest)}
