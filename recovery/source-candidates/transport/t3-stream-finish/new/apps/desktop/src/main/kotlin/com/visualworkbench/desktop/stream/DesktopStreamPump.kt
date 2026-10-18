package com.visualworkbench.desktop.stream

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

internal sealed interface HostPacketInfo {
    data class Tiles(val offer: ULong, val encodedBytes: UInt) : HostPacketInfo
    data class Original(val identity: CaptureImportDescriptor, val encodedBytes: UInt) : HostPacketInfo
    data object OriginalUnavailable : HostPacketInfo
    data object Idle : HostPacketInfo
}
internal interface HostPacket {
    val info: HostPacketInfo
    suspend fun payload(): ByteArray
    suspend fun close()
}
internal interface HostProducer {
    suspend fun poll(offer: ULong, admitted: Boolean, repair: Boolean): HostPacket
    suspend fun original(request: CaptureOriginalRequest): HostPacket
    suspend fun heartbeat(): HostPacket
    fun cancel()
    suspend fun close()
}
internal data class HostStreamState(val running: Boolean = false, val stopping: Boolean = false,
    val paused: Boolean = false, val publishedOffers: ULong = 0uL, val failure: String? = null)

/** One UI-owned run. The immutable factory captures the already approved source
 * and exact existing link; UI selection changes cannot redirect an active run. */
internal class DesktopStreamPump(private val scope: CoroutineScope) {
    private val mutable = MutableStateFlow(HostStreamState())
    val state: StateFlow<HostStreamState> = mutable.asStateFlow()
    private var work: Job? = null
    private var producer: HostProducer? = null
    private var publisherOwner: WorkbenchCapturePublisher? = null
    private var disposed = false
    private var cleanupFailed = false

    fun start(openProducer: suspend () -> HostProducer, openPublisher: suspend () -> WorkbenchCapturePublisher): Boolean {
        if (disposed || cleanupFailed || work != null) return false
        mutable.value = HostStreamState(running = true)
        val task = scope.launch(start = CoroutineStart.LAZY) {
            var native: HostProducer? = null
            var publisher: WorkbenchCapturePublisher? = null
            try {
                native = openProducer(); producer = native
                currentCoroutineContext().ensureActive()
                publisher = openPublisher(); publisherOwner = publisher
                pump(native, publisher)
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { mutable.value = mutable.value.copy(failure = "The approved source or paired stream became unavailable. Stop and start again after checking the source and connection.") }
            finally {
                // Production wrappers join uninterruptible native ownership.
                // Keep admission and UI indicator through both cleanup receipts.
                withContext(NonCancellable) {
                    var failure: Throwable? = null
                    try { native?.close(); producer = null } catch (error: Throwable) { failure = error }
                    try { publisher?.close(); publisherOwner = null } catch (error: Throwable) {
                        failure?.addSuppressed(error); if (failure == null) failure = error
                    }
                    cleanupFailed = failure != null
                    if (cleanupFailed) mutable.value = mutable.value.copy(failure = "Capture cleanup failed. Ownership remains sealed; retry Stop before replacing this project or connection.")
                }
            }
        }
        work = task
        task.invokeOnCompletion { scope.launch(NonCancellable) { if (work === task) { work = null; mutable.value = mutable.value.copy(running = false, stopping = false, paused = false) } } }
        task.start()
        return true
    }
    private suspend fun pump(native: HostProducer, publisher: WorkbenchCapturePublisher) {
        var offer = 0uL
        var admitted = false
        var repair = 0uL
        var answeredOriginal = 0uL
        while (currentCoroutineContext().isActive) {
            val signal = publisher.signal()
            if (signal.closed) throw SessionFailure(SessionFailureKind.Closed)
            mutable.value = mutable.value.copy(paused = signal.paused)
            val request = signal.original
            if (request != null && request.sequence > answeredOriginal) {
                consume(native.original(request)) { packet, bytes ->
                    when (val info = packet.info) {
                        is HostPacketInfo.Original -> {
                            require(bytes.size.toULong() == info.encodedBytes.toULong())
                            publisher.provideOriginal(request.sequence, info.identity, bytes)
                        }
                        HostPacketInfo.OriginalUnavailable -> {
                            require(bytes.isEmpty()); publisher.originalUnavailable(request.sequence)
                        }
                        else -> throw SessionFailure(SessionFailureKind.Invalid)
                    }
                }
                answeredOriginal = request.sequence
            } else if (signal.paused) {
                consume(native.heartbeat()) { packet, bytes ->
                    require(packet.info == HostPacketInfo.Idle && bytes.isEmpty())
                }
                delay(250)
            } else {
                val needRepair = signal.repairSequence > repair
                consume(native.poll(offer, admitted, needRepair)) { packet, bytes ->
                    // Feedback is consumed exactly once even when no new offer.
                    offer = 0uL; admitted = false
                    when (val info = packet.info) {
                        is HostPacketInfo.Tiles -> {
                            require(info.offer != 0uL && bytes.size.toULong() == info.encodedBytes.toULong())
                            offer = info.offer
                            admitted = publisher.publishFrame(bytes)
                            if (admitted) mutable.value = mutable.value.copy(publishedOffers = mutable.value.publishedOffers + 1uL)
                        }
                        HostPacketInfo.Idle -> require(bytes.isEmpty())
                        else -> throw SessionFailure(SessionFailureKind.Invalid)
                    }
                }
                repair = signal.repairSequence
                delay(5) // No busy-poll loop; helper owns actual adaptive cadence.
            }
        }
    }
    private suspend fun consume(packet: HostPacket, action: suspend (HostPacket, ByteArray) -> Unit) {
        var bytes: ByteArray? = null
        try { bytes = packet.payload(); action(packet, bytes) }
        finally {
            // No queue retains application arrays. Native publication joins its
            // own decoding/admission even if the caller was cancelled.
            bytes?.fill(0); bytes = null
            withContext(NonCancellable) { packet.close() }
        }
    }
    suspend fun stop(): Unit = withContext(NonCancellable) {
        mutable.value = mutable.value.copy(stopping = true)
        val active = work
        var failure: Throwable? = null
        try { producer?.cancel() } catch (error: Throwable) { failure = error }
        active?.cancelAndJoin()
        // A failed earlier close retains both owners. Retry their actual native
        // joins here before allowing link/project retirement or another run.
        try { producer?.close(); producer = null } catch (error: Throwable) { failure?.addSuppressed(error); if (failure == null) failure = error }
        try { publisherOwner?.close(); publisherOwner = null } catch (error: Throwable) { failure?.addSuppressed(error); if (failure == null) failure = error }
        cleanupFailed = producer != null || publisherOwner != null
        if (active === work) work = null
        mutable.value = mutable.value.copy(running = false, stopping = false)
        failure?.let { throw it }
    }
    suspend fun close(): Unit = withContext(NonCancellable) { disposed = true; stop() }
}