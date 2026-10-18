package com.visualworkbench.shared

import com.visualworkbench.bindings.core.CaptureStream as NStream
import com.visualworkbench.bindings.core.CaptureDraw as NDraw
import com.visualworkbench.bindings.core.CaptureBackground as NBackground
import com.visualworkbench.bindings.core.CapturePublisher as NPublisher
import com.visualworkbench.bindings.core.CapturePublishSource as NSource
import com.visualworkbench.bindings.core.CaptureImportDescriptor as NOriginal
import com.visualworkbench.bindings.core.StreamFrameInfo as NFrame
import com.visualworkbench.bindings.core.StreamSignal as NSignal
import com.visualworkbench.bindings.core.StreamStage as NStage
import com.visualworkbench.bindings.core.CapturePublicationSignal as NPublication
import com.visualworkbench.bindings.core.SessionException
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

private inline fun <T> captureNow(block: () -> T): T = try { block() }
catch (error: SessionException) { throw sessionFailure(error) }
catch (_: IllegalStateException) { throw SessionFailure(SessionFailureKind.Closed) }
private suspend fun <T> captureCall(block: suspend () -> T): T = withContext(Dispatchers.Default) {
    try { block() } catch (error: SessionException) { throw sessionFailure(error) }
    catch (_: IllegalStateException) { throw SessionFailure(SessionFailureKind.Closed) }
}
internal fun streamFrame(value: NFrame): CaptureStreamFrame = with(value) { CaptureStreamFrame(captureSessionId, frameId, sourceGeneration,
    width, height, hostX, hostY, dpiScale, geometryRevision, timestampNs, capturedAtMs, platform) }
private fun NSignal.common(): CaptureStreamSignal = CaptureStreamSignal(sequence, connectionEpoch, sourceGeneration,
    frameSequence, when (stage) {
        NStage.AWAITING_SOURCE -> CaptureStreamStage.AwaitingSource; NStage.LIVE -> CaptureStreamStage.Live
        NStage.FREEZING -> CaptureStreamStage.Freezing; NStage.FROZEN_LOSSLESS -> CaptureStreamStage.FrozenLossless
        NStage.FROZEN_DEGRADED -> CaptureStreamStage.FrozenDegraded; NStage.SAVED -> CaptureStreamStage.Saved
        NStage.SOURCE_CLOSED -> CaptureStreamStage.SourceClosed; NStage.CLOSED -> CaptureStreamStage.Closed
    }, width, height, retainedBytes, failure)
private fun NPublication.common(): CapturePublicationSignal = CapturePublicationSignal(sequence, connectionEpoch,
    sourceGeneration, paused, closed, repairSequence, original?.let { CaptureOriginalRequest(it.sequence, it.draftId, streamFrame(it.frame)) }, failure)
private fun CapturePublicationSource.native(): NSource = NSource(explicitOwnerAction, captureSessionId, sourceKind,
    windowHandle, monitorId, hostX, hostY, width, height, dpiScale, geometryRevision, observedTimestampNs, targetFps, interaction, platform)
private fun CaptureImportDescriptor.native(): NOriginal = NOriginal(explicitOwnerAction, captureSessionId, frameId,
    geometryRevision, platform, sourceKind, physicalX, physicalY, width, height, dpiScale, monotonicTimestampNs,
    capturedAtMs, windowHandle, monitorId, expectedSourceAssetId)

/** Reuses the checked borrowed link accessor; no parallel trust/runtime owner. */
public actual suspend fun ProjectLink.openCaptureStream(memoryBudgetBytes: ULong): WorkbenchCaptureStream {
    currentCoroutineContext().ensureActive()
    val access = this as? NativeFocusAccess ?: throw SessionFailure(SessionFailureKind.Invalid)
    return joinedCaptureCall(release = { it: WorkbenchCaptureStream -> it.close() }) {
        captureCall { NativeCaptureStream(access.focusHandle().openCaptureStream(memoryBudgetBytes)) }
    }
}
public actual suspend fun ProjectLink.openCapturePublisher(source: CapturePublicationSource,
    memoryBudgetBytes: ULong): WorkbenchCapturePublisher {
    currentCoroutineContext().ensureActive()
    val access = this as? NativeFocusAccess ?: throw SessionFailure(SessionFailureKind.Invalid)
    val descriptor = source.native()
    return joinedCaptureCall(release = { it: WorkbenchCapturePublisher -> it.close() }) {
        captureCall { NativeCapturePublisher(access.focusHandle().openCapturePublisher(descriptor, memoryBudgetBytes)) }
    }
}
internal class NativeCaptureStream(private val handle: NStream) : WorkbenchCaptureStream {
    private val owner = CaptureNativeOwner()
    internal suspend fun <T> withNative(block: suspend (NStream) -> T): T = owner.call { block(handle) }
    override val signals: Flow<CaptureStreamSignal> = flow {
        var sequence = 0uL
        while (currentCoroutineContext().isActive) {
            val next = owner.call { captureCall { handle.waitSignal(sequence).common() } }
            if (next.sequence > sequence) { sequence = next.sequence; emit(next) }
        }
    }
    override fun signal(): CaptureStreamSignal = owner.now { captureNow { handle.signal().common() } }
    override suspend fun beginDraw(): CaptureStreamDraw? = owner.call {
        joinedCaptureCall(release = { it: CaptureStreamDraw? -> it?.close() }) {
            captureCall { handle.beginDraw()?.let(::NativeCaptureDraw) }
        }
    }
    override suspend fun frozenBackground(): CaptureFrozenBackground? = owner.call {
        joinedCaptureCall(release = { it: CaptureFrozenBackground? -> it?.close() }) {
            captureCall { handle.frozenBackground()?.let(::NativeCaptureBackground) }
        }
    }
    override suspend fun firstContact(draftId: String): CaptureFreezeReceipt? = owner.call {
        joinedCaptureCall(release = { _: CaptureFreezeReceipt? ->
            // The caller never acquired a gesture. Close the stream instead of
            // manufacturing a resume while a real gesture might still exist.
            captureCall { handle.closeStream() }
        }) { captureCall { handle.firstContact(draftId)?.let { CaptureFreezeReceipt(it.draftId, it.frameId, it.captureSessionId, it.sourceGeneration) } } }
    }
    override suspend fun resumeAfterDraftDisposed(): String? = owner.call {
        joinedCaptureCall { captureCall { handle.resumeAfterDraftDisposed() } }
    }
    override suspend fun close(): Unit = owner.close({ captureCall { handle.closeStream() } }, handle::destroy)
}
private class NativeCaptureDraw(private val handle: NDraw) : CaptureStreamDraw {
    private val owner = CaptureNativeOwner()
    override val frame: CaptureStreamFrame get() = owner.now { captureNow { streamFrame(handle.info()) } }
    override suspend fun rgbaRows(firstRow: UInt, rowCount: UInt): ByteArray = owner.call {
        joinedCaptureCall { captureCall { handle.rgbaRows(firstRow, rowCount) } }
    }
    override suspend fun presented(actuallyShown: Boolean): Unit = owner.call {
        joinedCaptureCall { captureCall { handle.presented(actuallyShown) } }
    }
    // Destruction of an unsettled native ticket closes its lease. Never invent
    // false merely because a cancelled frame callback was not observed here.
    override suspend fun close(): Unit = owner.close({}, handle::destroy)
}
internal class NativeCaptureBackground(private val handle: NBackground) : CaptureFrozenBackground {
    private val owner = CaptureNativeOwner()
    internal suspend fun <T> withNative(block: suspend (NBackground) -> T): T = owner.call { block(handle) }
    override val frame: CaptureStreamFrame get() = owner.now { captureNow { streamFrame(handle.info()) } }
    override val draftId: String get() = owner.now { captureNow { handle.draftId() } }
    override val lossless: Boolean get() = owner.now { captureNow { handle.isLossless() } }
    override val originalAssetId: String? get() = owner.now { captureNow { handle.originalAssetId() } }
    override suspend fun rgbaRows(firstRow: UInt, rowCount: UInt): ByteArray = owner.call {
        joinedCaptureCall { captureCall { handle.rgbaRows(firstRow, rowCount) } }
    }
    override suspend fun close(): Unit = owner.close({}, handle::destroy)
}
private class NativeCapturePublisher(private val handle: NPublisher) : WorkbenchCapturePublisher {
    private val owner = CaptureNativeOwner()
    override val signals: Flow<CapturePublicationSignal> = flow {
        var sequence = 0uL
        while (currentCoroutineContext().isActive) {
            val next = owner.call { captureCall { handle.waitSignal(sequence).common() } }
            if (next.sequence > sequence) { sequence = next.sequence; emit(next) }
        }
    }
    override fun signal(): CapturePublicationSignal = owner.now { captureNow { handle.signal().common() } }
    override suspend fun publishFrame(encoded: ByteArray): Boolean = owner.call {
        // Joined through actual admission: the host packet's accounting lease
        // cannot be released by a cancelled Kotlin waiter before this returns.
        joinedCaptureCall { captureCall { handle.publishFrame(encoded) } }
    }
    override suspend fun provideOriginal(requestSequence: ULong, identity: CaptureImportDescriptor, encoded: ByteArray): Unit = owner.call {
        val exact = identity.native()
        joinedCaptureCall { captureCall { handle.provideOriginal(requestSequence, exact, encoded) } }
    }
    override suspend fun originalUnavailable(requestSequence: ULong): Unit = owner.call {
        joinedCaptureCall { captureCall { handle.originalUnavailable(requestSequence) } }
    }
    override suspend fun close(): Unit = owner.close({ captureCall { handle.closePublisher() } }, handle::destroy)
}
public actual suspend fun ProjectLink.captureRegionSignal():CaptureRegionSignal {
    val access=this as? NativeFocusAccess ?: throw SessionFailure(SessionFailureKind.Invalid)
    return captureCall { val value=access.focusHandle().captureRegionSignal()
        CaptureRegionSignal(value.sequence,value.connectionEpoch,value.requestId,value.outgoingRequestId,value.status) }
}
public actual suspend fun ProjectLink.requestCaptureRegion(requestId:String):Unit {
    currentCoroutineContext().ensureActive()
    val access=this as? NativeFocusAccess ?: throw SessionFailure(SessionFailureKind.Invalid)
    joinedCaptureCall { captureCall { access.focusHandle().requestCaptureRegion(requestId) } }
}
public actual suspend fun ProjectLink.resolveCaptureRegion(requestId:String,connectionEpoch:ULong,approve:Boolean):Unit {
    currentCoroutineContext().ensureActive()
    val access=this as? NativeFocusAccess ?: throw SessionFailure(SessionFailureKind.Invalid)
    joinedCaptureCall { captureCall { access.focusHandle().resolveCaptureRegion(requestId,connectionEpoch,approve) } }
}
