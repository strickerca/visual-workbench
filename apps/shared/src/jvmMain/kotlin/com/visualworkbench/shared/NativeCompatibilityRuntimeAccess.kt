package com.visualworkbench.shared

import com.visualworkbench.bindings.core.LiveSession
import com.visualworkbench.bindings.core.SessionException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

/** The normal link factory supplies this access. The foreign object is obtained
 * before bulk receipt bytes and retained through callback, fresh source/settings
 * validation and cancellation cleanup. No caller file can construct this owner. */
internal class NativeCompatibilityRuntimeAccess(private val handle:LiveSession):NativeCompatibilityObservationAccess {
    override suspend fun withCurrentReceipt(expected:RemoteTargetState,receiptSha256:String,
        body:suspend(NativeOwnedCompatibilityReceipt?)->RemoteCompatibilityAdmission?,
    ):RemoteCompatibilityAdmission? = read(expected,receiptSha256,body)
    override suspend fun withCurrentObservation(expected:RemoteTargetState,
        body:suspend(NativeOwnedCompatibilityReceipt?)->RemoteCompatibilityAdmission?,
    ):RemoteCompatibilityAdmission? = read(expected,null,body)
    private suspend fun read(expected:RemoteTargetState,receiptSha256:String?,
        body:suspend(NativeOwnedCompatibilityReceipt?)->RemoteCompatibilityAdmission?,
    ):RemoteCompatibilityAdmission? {
        // A synchronous bounded helper call may return after caller cancellation.
        // Keep acquisition and disposal in the same uncancellable bracket, so
        // dispatcher delivery can never abandon an already-created foreign owner.
        return try { withJoinedCompatibilityObservation(
            create={handle.remoteObserveCompatibility(RemoteNativeJson.binding(expected.binding))},
            release={owned->try { owned.releaseSource() } finally { owned.destroy() }},
        ) { owned ->
            val bytes=owned.receiptBytes()
            if(receiptSha256!=null&&NativeRemoteCompatibility.sha(bytes)!=receiptSha256)
                return@withJoinedCompatibilityObservation null
            val projected=NativeRemoteCompatibility.decode(expected,bytes)
            val result=body(NativeOwnedCompatibilityReceipt(expected,projected.toolId,
                projected.settingsDigest,bytes,projected.records))
            currentCoroutineContext().ensureActive()
            result.takeIf{owned.isCurrent()}
        } } catch(error:SessionException) { throw sessionFailure(error) }
    }
}
