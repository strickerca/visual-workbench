package com.visualworkbench.shared

/** DTOs for root-reviewed profile receipts. Neither constructing nor validating
 * these records creates native authority or advertised command capabilities. */
public data class RemoteCanvasSelector(
    public val frameworkId:String,
    public val className:String,
    public val automationId:String,
    public val controlType:Int,
    public val requireKeyboardFocus:Boolean,
)
public enum class RemoteProfileRoute { PairedKeys,FiniteWheel,FiniteCanvasClick }
public data class RemoteEditorActionObservation(
    public val action:RemoteEditorAction,
    public val route:RemoteProfileRoute,
    /** SHA-256 of the exact complete native batch retained in root's receipt. */
    public val nativeBatchDigest:String,
    public val inputSequence:ULong,
    public val acceptedQpc100ns:ULong,
    public val editorEffectObserved:Boolean,
    public val effectReceiptDigest:String?,
)
public data class RemoteEditorProfileEvidence(
    public val schemaVersion:UInt,
    public val identity:RemoteEditorIdentity,
    public val toolId:String,
    public val settingsDigest:String,
    public val selector:RemoteCanvasSelector,
    public val observations:List<RemoteEditorActionObservation>,
    public val guardLifecycleReceiptDigest:String,
    public val acceptanceReceiptDigest:String,
)
private fun String.profileDigest():Boolean=length==64&&all{it in '0'..'9'||it in 'a'..'f'}
private fun String.profileText(max:Int,empty:Boolean=false):Boolean=length in (if(empty)0 else 1)..max&&none{it.code<32||it.code==127}
public fun RemoteEditorProfileEvidence.validFor(current:RemoteEditorIdentity,tool:String,settings:String):Boolean {
    if(schemaVersion!=1u||!identity.valid()||!current.valid()||identity.copy(shortcutProfileDigest=null)!=current.copy(shortcutProfileDigest=null)||identity.editor==RemoteEditorKind.Unknown||toolId!=tool||settingsDigest!=settings||!toolId.profileText(128)||!settingsDigest.profileDigest())return false
    if(!selector.requireKeyboardFocus||!selector.frameworkId.profileText(128)||!selector.className.profileText(256)||!selector.automationId.profileText(256,true)||selector.controlType !in 50000..50100)return false
    if(!guardLifecycleReceiptDigest.profileDigest()||!acceptanceReceiptDigest.profileDigest()||observations.isEmpty()||observations.size>RemoteEditorAction.entries.size||observations.map{it.action}.distinct().size!=observations.size)return false
    return observations.all{it.action in remoteEditorActions(identity.editor)&&it.nativeBatchDigest.profileDigest()&&it.inputSequence!=0uL&&it.acceptedQpc100ns!=0uL&&it.editorEffectObserved&&it.effectReceiptDigest?.profileDigest()==true}
}
