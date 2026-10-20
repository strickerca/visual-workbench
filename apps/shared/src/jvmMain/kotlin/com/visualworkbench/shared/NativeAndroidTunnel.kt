package com.visualworkbench.shared

import com.visualworkbench.bindings.core.AndroidTunnelCapture as NCapture
import com.visualworkbench.bindings.core.AndroidTunnelCaptureGuard as NGuard
import com.visualworkbench.bindings.core.AndroidTunnelCaptureInfo as NInfo
import com.visualworkbench.bindings.core.AndroidTunnelElement as NElement
import com.visualworkbench.bindings.core.AndroidTunnelException as NFailure
import com.visualworkbench.bindings.core.AndroidTunnelHierarchy as NHierarchy
import com.visualworkbench.bindings.core.AndroidTunnelMode as NMode
import com.visualworkbench.bindings.core.AndroidTunnelOptions as NOptions
import com.visualworkbench.bindings.core.AndroidTunnelSession as NSession
import com.visualworkbench.bindings.core.AndroidTunnelSource as NSource
import com.visualworkbench.bindings.core.AndroidTunnelState as NState
import com.visualworkbench.bindings.core.AndroidTunnelTarget as NTarget
import com.visualworkbench.bindings.core.Cancellation as NCancel
import com.visualworkbench.bindings.core.CoreException as NCoreFailure
import com.visualworkbench.bindings.core.CreateFileImageProject
import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.androidTunnelAvailable as nativeAvailable
import com.visualworkbench.bindings.core.assertAndroidTunnelRetired as nativeAssertRetired
import com.visualworkbench.bindings.core.beginAndroidTunnelCaptureGuard as nativeBeginGuard
import com.visualworkbench.bindings.core.openAndroidTunnel as nativeOpen
import com.visualworkbench.bindings.core.retryAndroidTunnelRetirement as nativeRetryRetirement
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

private fun tunnelFailure(error: NFailure): TunnelFailure = TunnelFailure(when (error) {
    is NFailure.Invalid -> TunnelFailureKind.Invalid
    is NFailure.Consent -> TunnelFailureKind.Consent
    is NFailure.AmbiguousDevice -> TunnelFailureKind.AmbiguousDevice
    is NFailure.UnauthorizedDevice -> TunnelFailureKind.UnauthorizedDevice
    is NFailure.RuntimeUnavailable -> TunnelFailureKind.RuntimeUnavailable
    is NFailure.FeedbackLoop -> TunnelFailureKind.FeedbackLoop
    is NFailure.Stale -> TunnelFailureKind.Stale
    is NFailure.Limit -> TunnelFailureKind.Limit
    is NFailure.Cancelled -> TunnelFailureKind.Cancelled
    is NFailure.Timeout -> TunnelFailureKind.Timeout
    is NFailure.Storage -> TunnelFailureKind.Storage
    is NFailure.Platform -> TunnelFailureKind.Platform
    is NFailure.RetirementPending -> TunnelFailureKind.RetirementPending
    is NFailure.Closed -> TunnelFailureKind.Closed
})
private fun tunnelFailure(error: SessionFailure): TunnelFailure = TunnelFailure(when (error.kind) {
    SessionFailureKind.Backpressure -> TunnelFailureKind.Limit
    SessionFailureKind.Closed -> TunnelFailureKind.Closed
    SessionFailureKind.Cancelled -> TunnelFailureKind.Cancelled
    else -> TunnelFailureKind.Platform
})
private inline fun <T> tunnelNow(block: () -> T): T = try { block() }
catch (error: NFailure) { throw tunnelFailure(error) }
catch (error: NCoreFailure) { throw mapFailure(error) }
catch (error: SessionFailure) { throw tunnelFailure(error) }
catch (_: IllegalStateException) { throw TunnelFailure(TunnelFailureKind.Closed) }
private suspend fun <T> tunnelMapped(block: suspend () -> T): T = try { block() }
catch (error: NFailure) { throw tunnelFailure(error) }
catch (error: NCoreFailure) { throw mapFailure(error) }
catch (error: SessionFailure) { throw tunnelFailure(error) }
catch (_: IllegalStateException) { throw TunnelFailure(TunnelFailureKind.Closed) }
private suspend fun <T> tunnelCall(block: suspend () -> T): T =
    withContext(Dispatchers.Default) { tunnelMapped(block) }

private fun TunnelOptions.native(): NOptions = NOptions(explicitOwnerAction, runtimeDirectory, ownerAdbPath,
    selectedModel, selectedTransportId, sessionId)
private fun TunnelSource.native(): NSource = when (this) {
    is TunnelSource.Window -> NSource.Window(window)
    is TunnelSource.Monitor -> NSource.Monitor(monitor)
    is TunnelSource.Region -> NSource.Region(monitor, x, y, width, height)
}
private fun NState.common(): TunnelState = TunnelState(when (mode) {
    NMode.VIEW_ONLY -> TunnelMode.ViewOnly
    NMode.OWNER_CONTROL -> TunnelMode.OwnerControl
    NMode.RETIREMENT_PENDING -> TunnelMode.RetirementPending
    NMode.CLOSED -> TunnelMode.Closed
}, mirrorRevision)
private fun NTarget.common(): TunnelTarget = TunnelTarget(model, transportId, mirrorRevision)
private fun TunnelTarget.native(): NTarget = NTarget(model, transportId, mirrorRevision)
private fun NElement.common(): TunnelElement = TunnelElement(left, top, right, bottom, className, resourceId, description, text)
private fun NInfo.common(): TunnelCaptureInfo = TunnelCaptureInfo(captureSessionId, sourceGeneration,
    frameId, width, height, encodedBytes, assetId, imageStartedMs, imageFinishedMs, imageFinishedTimestampNs,
    when (hierarchy) {
        NHierarchy.PRESENT -> TunnelHierarchy.Present
        NHierarchy.MISSING -> TunnelHierarchy.Missing
        NHierarchy.STALE -> TunnelHierarchy.Stale
    }, hierarchyAgeMs, hierarchyStartedMs, hierarchyFinishedMs)

/** Independent factories have no ProjectLink parent. Keep admitted acquisitions
 * and failed close/destructor owners strongly reachable until actual settlement. */
private object TunnelRetirements {
    private val owners = linkedSetOf<TunnelRootSlot>()
    @Synchronized fun register(slot: TunnelRootSlot) {
        if (owners.size >= 16) throw TunnelFailure(TunnelFailureKind.Limit)
        owners.add(slot)
    }
    @Synchronized fun retired(slot: TunnelRootSlot) { owners.remove(slot) }
    @Synchronized fun retrying(): List<TunnelRootSlot> = owners.filter { it.needsRetry }
    @Synchronized fun hasOwners(): Boolean = owners.isNotEmpty()
}
private class TunnelRootSlot {
    private val ready = CompletableDeferred<Unit>()
    private val retirement = Mutex()
    private var resource: OwnedSession? = null
    @Volatile var needsRetry: Boolean = false; private set
    @Synchronized fun acquired(owner: OwnedSession) { resource = owner; ready.complete(Unit) }
    fun failed() { needsRetry = true; ready.complete(Unit) }
    fun retry() { needsRetry = true }
    fun retired() { TunnelRetirements.retired(this) }
    suspend fun close(): Unit = withContext(NonCancellable) {
        needsRetry = true
        ready.await()
        retirement.withLock {
            val owner = synchronized(this@TunnelRootSlot) { resource }
            owner?.close()
            synchronized(this@TunnelRootSlot) { resource = null }
            retired()
        }
    }
}

private suspend fun <T : OwnedSession> acquireTunnelOwner(
    create: suspend (NCancel, () -> Unit, () -> Unit) -> T,
): T = tunnelMapped {
    currentCoroutineContext().ensureActive()
    val slot = TunnelRootSlot()
    TunnelRetirements.register(slot) // Before cancellation or native allocation.
    var token: NCancel? = null
    try {
        val cancel = NCancel(); token = cancel
        val value = joinedCaptureCall(cancel = cancel::cancel, release = { _: T -> slot.close() }) {
            try { create(cancel, slot::retired, slot::retry).also(slot::acquired) }
            catch (error: Throwable) { slot.failed(); throw error }
        }
        // Destroy only after the producer/cancel hook settled; a failed token
        // destructor cannot turn an undelivered produced owner into a leak.
        token = null
        tunnelNow(cancel::destroy)
        currentCoroutineContext().ensureActive()
        value
    } catch (error: Throwable) {
        slot.failed()
        withContext(NonCancellable) {
            try { slot.close() } catch (cleanup: Throwable) { if (cleanup !== error) error.addSuppressed(cleanup) }
            val remaining = token; token = null
            try { remaining?.let { tunnelNow(it::destroy) } }
            catch (cleanup: Throwable) { if (cleanup !== error) error.addSuppressed(cleanup) }
        }
        throw error
    }
}

public actual fun androidTunnelAvailable(): Boolean = tunnelNow(::nativeAvailable)
public actual suspend fun openAndroidTunnel(options: TunnelOptions): WorkbenchAndroidTunnel =
    acquireTunnelOwner { cancel, retired, failed ->
        NativeAndroidTunnel(tunnelCall { nativeOpen(options.native(), cancel) }, retired, failed)
    }

public actual suspend fun retryAndroidTunnelRetirement(): Unit = tunnelMapped {
    joinedCaptureCall {
        var failure: Throwable? = null
        for (slot in TunnelRetirements.retrying()) try { slot.close() } catch (error: Throwable) {
            val prior = failure
            if (prior == null) failure = error else if (prior !== error) prior.addSuppressed(error)
        }
        // Native process/IO retirement is still attempted when an independent
        // generated-handle retirement remains failed and strongly retained.
        try { tunnelCall { nativeRetryRetirement() } } catch (error: Throwable) {
            val prior = failure
            if (prior == null) failure = error else if (prior !== error) prior.addSuppressed(error)
        }
        failure?.let { throw it }
    }
}
public actual fun assertAndroidTunnelRetired(): Unit = tunnelNow {
    nativeAssertRetired()
    if (TunnelRetirements.hasOwners()) throw TunnelFailure(TunnelFailureKind.RetirementPending)
}

private class NativeAndroidTunnel(private val handle: NSession, private val retired: () -> Unit,
    private val failed: () -> Unit) : WorkbenchAndroidTunnel, OwnedSession {
    private val owner = CaptureNativeOwner()
    private val children = CaptureChildren()
    private suspend fun <T> operation(block: suspend () -> T): T = tunnelMapped {
        owner.call { joinedCaptureCall(cancel = handle::cancel) { tunnelCall(block) } }
    }
    override suspend fun state(): TunnelState = operation { handle.state().common() }
    override suspend fun beginControlConfirmation(): TunnelTarget = operation { handle.beginControlConfirmation().common() }
    override suspend fun confirmControl(target: TunnelTarget): TunnelState = operation { handle.confirmControl(target.native()).common() }
    override suspend fun cancelControlConfirmation(): Unit = operation { handle.cancelControlConfirmation() }
    override suspend fun revokeControl(): TunnelState = operation { handle.revokeControl().common() }
    override suspend fun capture(): WorkbenchTunnelCapture = tunnelMapped {
        owner.call { children.acquire(cancel = handle::cancel) { childRetired ->
            NativeTunnelCapture(tunnelCall { handle.capture() }, childRetired, failed)
        } }
    }
    override fun cancel(): Unit = tunnelNow { owner.now { handle.cancel() } }
    override suspend fun close(): Unit = withContext(NonCancellable) {
        try {
            owner.close({
                var failure: Throwable? = null
                suspend fun release(block: suspend () -> Unit) {
                    try { block() } catch (error: Throwable) {
                        val prior = failure
                        if (prior == null) failure = error else if (prior !== error) prior.addSuppressed(error)
                    }
                }
                release { tunnelNow { handle.cancel() } }
                release { children.close() }
                release { tunnelCall { handle.closeSession() } }
                failure?.let { throw it }
            }, handle::destroy)
            retired()
        } catch (error: Throwable) { failed(); throw error }
    }
}

private class NativeTunnelCapture(private val handle: NCapture, private val retired: () -> Unit,
    private val failed: () -> Unit) : WorkbenchTunnelCapture, OwnedSession {
    private val owner = CaptureNativeOwner()
    override fun info(): TunnelCaptureInfo = tunnelNow { owner.now { handle.info().common() } }
    override fun elements(): List<TunnelElement> = tunnelNow { owner.now { handle.elements().map { it.common() } } }
    override suspend fun createProject(options: CreateFileProject): WorkbenchProject = tunnelMapped {
        owner.call {
            val acquired = acquireTunnelOwner { cancel, projectRetired, projectFailed ->
                NativeTunnelProject(tunnelCall { handle.createProject(CreateFileImageProject(options.path,
                    options.projectId, options.documentId, options.layerId, options.deviceId, options.title,
                    "", options.workDirectory, options.nowMs, options.memoryBudgetBytes, options.maxEncodedBytes,
                    options.maxScratchBytes), cancel) }, projectRetired, projectFailed)
            }
            try { acquired.transfer() }
            catch (error: Throwable) {
                withContext(NonCancellable) {
                    try { acquired.close() } catch (cleanup: Throwable) { if (cleanup !== error) error.addSuppressed(cleanup) }
                }
                throw error
            }
        }
    }
    override suspend fun close(): Unit = withContext(NonCancellable) {
        try { owner.close({ tunnelCall { handle.closeCapture() } }, handle::destroy); retired() }
        catch (error: Throwable) { failed(); throw error }
    }
}

/** A late/failed import stays retained independently of its capture. Successful
 * transfer returns the actual ordinary NativeProject, preserving its APIs. */
private class NativeTunnelProject(private val handle: ProjectSession, private val retired: () -> Unit,
    private val failed: () -> Unit) : OwnedSession {
    private val owner = CaptureNativeOwner()
    fun transfer(): WorkbenchProject = wrapNativeProject(handle).also { retired() }
    override suspend fun close(): Unit = withContext(NonCancellable) {
        try { owner.close({ tunnelCall { handle.closeSession() } }, handle::destroy); retired() }
        catch (error: Throwable) { failed(); throw error }
    }
}

public actual fun beginAndroidTunnelCaptureGuard(source: TunnelSource): WorkbenchTunnelCaptureGuard = tunnelNow {
    val slot = TunnelRootSlot()
    TunnelRetirements.register(slot)
    try {
        val guard = NativeTunnelGuard(nativeBeginGuard(source.native()), slot::retired, slot::retry)
        slot.acquired(object : OwnedSession {
            override suspend fun close(): Unit = tunnelCall { guard.close() }
        })
        guard
    } catch (error: Throwable) { slot.failed(); slot.retired(); throw error }
}

/** Native marker operations are synchronous. Holding this gate joins a check
 * before close/destroy, and a failed destructor is sticky across close retry. */
private class NativeTunnelGuard(private val handle: NGuard, private val retired: () -> Unit,
    private val failed: () -> Unit) : WorkbenchTunnelCaptureGuard {
    private var closing = false
    private var destroyed = false
    private var nativeClosed = false
    private var destructionFailure: Throwable? = null
    @Synchronized override fun check(): Unit = tunnelNow {
        if (closing) throw TunnelFailure(TunnelFailureKind.Closed)
        handle.check()
    }
    @Synchronized override fun close(): Unit {
        closing = true
        try {
            destructionFailure?.let { throw it }
            if (destroyed) return
            if (!nativeClosed) { tunnelNow { handle.closeGuard() }; nativeClosed = true }
            try { handle.destroy() } catch (error: Throwable) { destructionFailure = error; throw error }
            destroyed = true; retired()
        } catch (error: Throwable) { failed(); throw error }
    }
}
