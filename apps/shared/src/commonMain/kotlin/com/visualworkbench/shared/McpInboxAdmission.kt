package com.visualworkbench.shared

internal fun ownedMcpSubmission(value:McpInboxSubmission):McpInboxSubmission {
    if(value.receiptId.length!=36||value.packageId.length!=36||value.manifestSha256.length!=64||value.connection.length!=32||
        value.createdAtMs<0||(value.text==null)==(value.png==null))throw PackageFailure(PackageFailureKind.Invalid)
    fun text(value:String){if(value.length>32768||value.encodeToByteArray().size>32768||'\u0000' in value)throw PackageFailure(PackageFailureKind.Limit)}
    text(value.note);value.text?.let{text(it);if(it.isEmpty())throw PackageFailure(PackageFailureKind.Invalid)}
    if(value.png?.let{it.isEmpty()||it.size>4*1024*1024}==true)throw PackageFailure(PackageFailureKind.Limit)
    return value.copy(png=value.png?.copyOf())
}
