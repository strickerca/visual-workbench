package com.visualworkbench.shared

/** HIL-only caller. Native feature is disabled in ordinary APK/distribution builds.
 * Arming precedes Down; abort never pauses or closes input owners. */
public fun armRemoteIntegrationCarrierFault(link:ProjectLink,runId:String,binding:RemoteTargetBinding):Unit = integrationFaultCall {
    require(runId.matches(Regex("[a-f0-9]{32}")))
    val handle=(link as? NativeFocusAccess)?.focusHandle()?:throw SessionFailure(SessionFailureKind.Invalid)
    handle.remoteIntegrationArmCarrierFault(runId,RemoteNativeJson.binding(binding))
}
public fun abortRemoteIntegrationCarrier(link:ProjectLink,runId:String,binding:RemoteTargetBinding):Unit = integrationFaultCall {
    require(runId.matches(Regex("[a-f0-9]{32}")))
    val handle=(link as? NativeFocusAccess)?.focusHandle()?:throw SessionFailure(SessionFailureKind.Invalid)
    handle.remoteIntegrationAbortCarrier(runId,RemoteNativeJson.binding(binding))
}

private fun integrationFaultCall(block:()->Unit):Unit = try { block() } catch(error:com.visualworkbench.bindings.core.SessionException) { throw sessionFailure(error) }
