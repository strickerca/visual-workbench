package com.visualworkbench.desktop.stream

import com.visualworkbench.bindings.host.CaptureService
import com.visualworkbench.bindings.host.CaptureSelection
import com.visualworkbench.bindings.host.HostCaptureStream
import com.visualworkbench.bindings.host.HostCaptureStreamOptions
import com.visualworkbench.bindings.host.HostStreamPacket
import com.visualworkbench.bindings.host.HostStreamPacketInfo
import com.visualworkbench.shared.*
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock

private val nativeCalls = CoroutineScope(SupervisorJob() + Dispatchers.Default)
private val nativeSlots = Semaphore(2)

/** Bounded independent producer, not a cancellable withContext resource return.
 * A cancelled waiter signals the exact helper and joins it before releasing the
 * output packet/slot; no JNI result can fall between dispatcher handoff and owner. */
internal suspend fun <T> joinedHostStreamCall(cancel: () -> Unit = {}, release: suspend (T) -> Unit = {}, block: suspend () -> T): T {
    currentCoroutineContext().ensureActive()
    if (!nativeSlots.tryAcquire()) throw SessionFailure(SessionFailureKind.Backpressure)
    try {
        val owned = AtomicReference<Result<T>?>(null)
        val job = nativeCalls.async { runCatching { block() }.also(owned::set).getOrThrow() }
        try {
            val value = job.await(); currentCoroutineContext().ensureActive(); owned.set(null); return value
        } catch (error: CancellationException) {
            try { cancel() } catch (signal: Throwable) { error.addSuppressed(signal) }
            withContext(NonCancellable) {
                try { job.await() } catch (_: Throwable) { }
                owned.getAndSet(null)?.let { if (it.isSuccess) try { release(it.getOrThrow()) } catch (cleanup: Throwable) { error.addSuppressed(cleanup) } }
            }
            throw error
        }
    } finally { nativeSlots.release() }
}

/** Exact mapping from the PC-approved picker selection; a phone hint is never
 * accepted here in place of that local selection/approval. Native revalidates
 * physical geometry and process identity before the persistent helper starts. */
internal fun streamPublicationSource(selection: CaptureSelection, captureSessionId: String,
    geometryRevision: UInt, targetFps: UInt, explicitOwnerAction: Boolean): CapturePublicationSource {
    require(explicitOwnerAction)
    return when (selection) {
        is CaptureSelection.Window -> selection.target.let { t ->
            require(t.observedNs <= Long.MAX_VALUE.toULong())
            CapturePublicationSource(true, captureSessionId, "window", t.window, "", t.clientX, t.clientY,
                t.clientWidth, t.clientHeight, t.dpi.toDouble() / 96.0, geometryRevision, t.observedNs.toLong(), targetFps)
        }
        is CaptureSelection.Monitor -> selection.target.let { t ->
            require(t.observedNs <= Long.MAX_VALUE.toULong())
            CapturePublicationSource(true, captureSessionId, "monitor", 0uL, t.device, t.x, t.y,
                t.width, t.height, t.dpi.toDouble() / 96.0, geometryRevision, t.observedNs.toLong(), targetFps)
        }
        is CaptureSelection.Region -> selection.target.let { t ->
            require(t.observedNs <= Long.MAX_VALUE.toULong())
            CapturePublicationSource(true, captureSessionId, "region", 0uL, t.device, selection.x, selection.y,
                selection.width, selection.height, t.dpi.toDouble() / 96.0, geometryRevision, t.observedNs.toLong(), targetFps)
        }
    }
}
internal suspend fun openNativeHostStream(service: CaptureService, selection: CaptureSelection,
    source: CapturePublicationSource): HostProducer {
    // Duplicate sources supplied independently cannot widen or redirect capture.
    require(source == streamPublicationSource(selection, source.captureSessionId, source.geometryRevision, source.targetFps, source.explicitOwnerAction))
    val options = HostCaptureStreamOptions(true, selection, source.captureSessionId, source.geometryRevision,
        source.targetFps, 8u, 300_000_000uL, 640uL * 1024uL * 1024uL, 193uL * 1024uL * 1024uL)
    return joinedHostStreamCall(release = { it: HostProducer -> it.close() }) {
        NativeHostProducer(service.openCaptureStream(options))
    }
}
private class NativeHostProducer(private val handle: HostCaptureStream) : HostProducer {
    private val closing = AtomicBoolean(false)
    private val closeGate = Mutex()
    private var destroyed = false
    private fun checkOpen() { if (closing.get()) throw SessionFailure(SessionFailureKind.Closed) }
    override suspend fun poll(offer: ULong, admitted: Boolean, repair: Boolean): HostPacket = request { handle.poll(offer, admitted, repair) }
    override suspend fun original(requested: CaptureOriginalRequest): HostPacket = request { handle.original(requested.frame.captureSessionId, requested.frame.frameId) }
    override suspend fun heartbeat(): HostPacket = request { handle.heartbeat() }
    private suspend fun request(block: suspend () -> HostStreamPacket): HostPacket {
        checkOpen()
        return joinedHostStreamCall(cancel = ::cancel, release = { it: HostPacket -> it.close() }) {
            checkOpen(); NativeHostPacket(block())
        }
    }
    override fun cancel() { if (!destroyed) { closing.set(true); handle.cancel() } }
    override suspend fun close(): Unit = withContext(NonCancellable) {
        closeGate.withLock {
            if (destroyed) return@withLock
            closing.set(true)
            withContext(Dispatchers.Default) { handle.closeHostStream() }
                handle.destroy(); destroyed = true
        }
    }
}
private class NativeHostPacket(private val handle: HostStreamPacket) : HostPacket {
    private val destroyed = AtomicBoolean(false)
    override val info: HostPacketInfo get() {
        check(!destroyed.get())
        return when (val value = handle.info()) {
            is HostStreamPacketInfo.Tiles -> HostPacketInfo.Tiles(value.offerSequence, value.encodedBytes)
            is HostStreamPacketInfo.Original -> value.identity.let { i -> HostPacketInfo.Original(
                CaptureImportDescriptor(true, i.captureSessionId, i.frameId, i.geometryRevision, i.platform,
                    i.sourceKind, i.physicalX, i.physicalY, i.width, i.height, i.dpiScale, i.timestampNs,
                    i.capturedAtMs, i.windowHandle, i.monitorId, i.sourceAssetId), value.encodedBytes) }
            HostStreamPacketInfo.OriginalUnavailable -> HostPacketInfo.OriginalUnavailable
            is HostStreamPacketInfo.Idle -> HostPacketInfo.Idle
        }
    }
    override suspend fun payload(): ByteArray {
        check(!destroyed.get())
        // Bounded payload move runs on the producer dispatcher; caller cannot
        // close the accounting lease until this exact method has settled.
        return joinedHostStreamCall { handle.takePayload() }
    }
    override suspend fun close(): Unit = withContext(NonCancellable) {
        if (destroyed.compareAndSet(false, true)) handle.destroy()
    }
}
/** The separate stream helper is admitted from the packaged hash inventory.
 * Its executable read lease stays alive through actual stream termination. */
internal suspend fun openOwnedHostStream(selection: CaptureSelection, source: CapturePublicationSource,
    openService: suspend () -> CaptureService): HostProducer {
    var service: CaptureService? = null
    var producer: HostProducer? = null
    var delivered = false
    try {
        val opened = openService(); service = opened
        val stream = openNativeHostStream(opened, selection, source); producer = stream
        currentCoroutineContext().ensureActive()
        val owner = object : HostProducer by stream {
            private var retired = false
            override suspend fun close(): Unit = withContext(NonCancellable) {
                if (!retired) { stream.close(); opened.destroy(); retired = true }
            }
        }
        delivered = true
        return owner
    } finally { if (!delivered) withContext(NonCancellable) { producer?.close(); service?.destroy() } }
}
