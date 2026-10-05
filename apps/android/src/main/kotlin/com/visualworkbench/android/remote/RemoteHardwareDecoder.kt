package com.visualworkbench.android.remote

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.Parcel
import android.view.Surface
import java.nio.ByteBuffer
import java.util.UUID
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.locks.ReentrantLock
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext

data class DecoderScope(val connectionEpoch: ULong, val captureSessionId: String,
    val sourceGeneration: ULong, val targetToken: String, val geometryRevision: UInt)
data class DecoderConfig(val scope: DecoderScope, val generation: ULong,
    val codedWidth: Int, val codedHeight: Int, val visibleWidth: Int, val visibleHeight: Int,
    val vps: ByteArray, val sps: ByteArray, val pps: ByteArray)
data class DecoderFrame(val ticket: ULong, val scope: DecoderScope, val generation: ULong,
    val frameId: ULong, val ptsUs: Long, val annexB: ByteArray,
    val enqueuedLocalNanos: Long, val isIdr: Boolean)

data class DecoderVendorCapability(val name: String, val type: Int?,
    val requested: Boolean, val configureAccepted: Boolean)
data class DecoderAdmission(val codecName: String, val hardwareAccelerated: Boolean,
    val lowLatencyAdvertised: Boolean, val lowLatencyRequested: Boolean,
    val lowLatencyConfigureAccepted: Boolean, val sizeAnd30FpsAdvertised: Boolean,
    val vendors: List<DecoderVendorCapability>)
/** Phone-local observations only. Output release, surface rendered callback and
 * callback arrival are distinct; none is a host/editor/photon acknowledgement. */
data class DecoderFrameTiming(val ownerId: String, val scope: DecoderScope, val generation: ULong,
    val ticket: ULong, val frameId: ULong, val ptsUs: Long, val enqueuedLocalNanos: Long,
    val outputReleaseRequestedLocalNanos: Long, val outputReleaseReturnedLocalNanos: Long,
    val renderedLocalNanos: Long, val callbackLocalNanos: Long)
sealed interface DecoderStatus {
    data object Starting : DecoderStatus
    data class Ready(val admission: DecoderAdmission) : DecoderStatus
    data class RecoveryRequired(val reason: String) : DecoderStatus
    data object Retiring : DecoderStatus
    data object Retired : DecoderStatus
}
sealed interface DecoderOpenResult {
    data class Started(val decoder: RemoteHardwareDecoder) : DecoderOpenResult
    data class Refused(val reason: String) : DecoderOpenResult
}
sealed interface DecoderQueueResult {
    data object Accepted : DecoderQueueResult
    data class Refused(val reason: String) : DecoderQueueResult
    data class RecoveryRequired(val reason: String) : DecoderQueueResult
}
sealed interface DecoderRetirement {
    data object Retired : DecoderRetirement
    data class Pending(val reason: String) : DecoderRetirement
}

internal const val MAX_REMOTE_AU: Int = 6 * 1024 * 1024
internal const val MAX_REMOTE_CONFIG: Int = 8 * 1024
internal const val FRAME_DEADLINE_NANOS: Long = 500_000_000
internal const val START_DEADLINE_MILLIS: Long = 3000

internal fun validDecoderConfig(config: DecoderConfig): Boolean {
    val s = config.scope
    return s.connectionEpoch != 0uL && s.sourceGeneration != 0uL && s.geometryRevision != 0u &&
        s.captureSessionId.isNotBlank() && s.captureSessionId.length <= 128 &&
        s.targetToken.isNotBlank() && s.targetToken.length <= 256 && config.generation != 0uL &&
        config.codedWidth in 2..4096 && config.codedHeight in 2..4096 &&
        config.codedWidth % 2 == 0 && config.codedHeight % 2 == 0 &&
        config.visibleWidth in (config.codedWidth - 1)..config.codedWidth &&
        config.visibleHeight in (config.codedHeight - 1)..config.codedHeight &&
        listOf(config.vps, config.sps, config.pps).all { it.size in 2..MAX_REMOTE_CONFIG } &&
        config.vps.size + config.sps.size + config.pps.size <= MAX_REMOTE_CONFIG
}

/** Closed callback diagnostics only. All values are relative and clamped; no
 * scope/ticket/frame IDs, absolute clocks or codec body are exposed. Mask bits:
 * 1 absent ticket, 2 absent release request, 4 absent release return, 8 PTS,
 * 16 render before request, 32 callback before render, 64 callback before enqueue,
 * 128 unchanged 500ms age predicate. This creates no rendered authority. */
data class DecoderRenderIdentity(val failureMask: Int, val ptsDeltaUs: Long?,
    val renderedFromRequestNanos: Long?, val callbackFromRenderedNanos: Long,
    val callbackFromEnqueuedNanos: Long?) {
    fun boundedText(): String = "mask=$failureMask;pts_us=${ptsDeltaUs ?: "none"};render_request_ns=${renderedFromRequestNanos ?: "none"};callback_render_ns=$callbackFromRenderedNanos;callback_enqueue_ns=${callbackFromEnqueuedNanos ?: "none"}"
}
private fun renderDiagnosticDelta(value: Long, baseline: Long): Long {
    val difference = try { Math.subtractExact(value, baseline) }
        catch (_: ArithmeticException) { if (value >= baseline) Long.MAX_VALUE else Long.MIN_VALUE }
    return difference.coerceIn(-1_000_000_000_000L, 1_000_000_000_000L)
}

/** Pure bounded ticket/PTS state. Native preflight validates HEVC syntax and the
 * config; this second fence validates ownership and callback identity. */
internal class DecoderTickets(private val config: DecoderConfig) {
    var outstanding: DecoderFrame? = null; private set
    private var first = true
    private var lastPts = -1L
    private var lastFrame = 0uL
    private var lastTicket = 0uL
    var releaseRequestedNanos: Long? = null; private set
    var outputReleaseReturnedNanos: Long? = null; private set
    fun admit(frame: DecoderFrame, now: Long): String? {
        if (outstanding != null) return "QueueLoss"
        if (frame.scope != config.scope || frame.generation != config.generation) return "StaleScope"
        if (frame.ticket == 0uL || frame.ticket == lastTicket || frame.frameId <= lastFrame ||
            frame.ptsUs < 0 || frame.ptsUs <= lastPts || frame.annexB.size !in 1..MAX_REMOTE_AU ||
            frame.enqueuedLocalNanos < 0 || now < frame.enqueuedLocalNanos ||
            now - frame.enqueuedLocalNanos >= FRAME_DEADLINE_NANOS) return "InvalidFrame"
        if (first && !frame.isIdr) return "IdrRequired"
        outstanding = frame.copy(annexB = frame.annexB.copyOf())
        first = false; lastPts = frame.ptsUs; lastFrame = frame.frameId; lastTicket = frame.ticket
        releaseRequestedNanos = null; outputReleaseReturnedNanos = null
        return null
    }
    fun released(pts: Long, requested: Long, returned: Long): Boolean {
        val frame = outstanding ?: return false
        if (frame.ptsUs != pts || requested < frame.enqueuedLocalNanos || returned < requested) return false
        releaseRequestedNanos = requested; outputReleaseReturnedNanos = returned
        return true
    }
    fun renderIdentity(pts: Long, rendered: Long, callback: Long): DecoderRenderIdentity {
        val frame = outstanding
        val requested = releaseRequestedNanos
        var mask = 0
        if (frame == null) mask = mask or 1
        if (requested == null) mask = mask or 2
        if (outputReleaseReturnedNanos == null) mask = mask or 4
        if (frame != null && pts != frame.ptsUs) mask = mask or 8
        if (requested != null && rendered < requested) mask = mask or 16
        if (callback < rendered) mask = mask or 32
        if (frame != null && callback < frame.enqueuedLocalNanos) mask = mask or 64
        // Same original subtraction/age predicate as rendered(); diagnostics do
        // not change timestamp admission or consume the outstanding ticket.
        if (frame != null && callback - frame.enqueuedLocalNanos >= FRAME_DEADLINE_NANOS) mask = mask or 128
        return DecoderRenderIdentity(mask, frame?.let { renderDiagnosticDelta(pts, it.ptsUs) },
            requested?.let { renderDiagnosticDelta(rendered, it) },
            renderDiagnosticDelta(callback, rendered), frame?.let { renderDiagnosticDelta(callback, it.enqueuedLocalNanos) })
    }
    fun rendered(pts: Long, rendered: Long, callback: Long): DecoderFrame? {
        val frame = outstanding ?: return null
        val requested = releaseRequestedNanos ?: return null
        if (outputReleaseReturnedNanos == null || pts != frame.ptsUs ||
            rendered < requested || callback < rendered || callback < frame.enqueuedLocalNanos ||
            callback - frame.enqueuedLocalNanos >= FRAME_DEADLINE_NANOS) return null
        outstanding = null
        return frame
    }
    fun retire() { outstanding = null; releaseRequestedNanos = null; outputReleaseReturnedNanos = null }
}

internal sealed interface DecoderOutput {
    data object None : DecoderOutput
    data object FormatChanged : DecoderOutput
    data class Buffer(val index: Int, val ptsUs: Long, val flags: Int) : DecoderOutput
}
internal interface DecoderEngine {
    fun start(config: DecoderConfig, surface: Surface, callbacks: Handler,
        onRendered: (Long, Long, Long) -> Unit): DecoderAdmission
    fun queue(bytes: ByteArray, ptsUs: Long): Boolean
    fun output(): DecoderOutput
    fun outputDimensions(): Pair<Int, Int>
    fun discardOutput(index: Int)
    fun renderOutput(index: Int, requestedLocalNanos: Long)
    fun stop()
    fun release()
}

/** Samples the phone-local request and actual return separately. Media PTS is
 * an identity in microseconds; it never chooses the Surface scheduling clock.
 * Exceptions retain the original failure and do not create a release receipt. */
internal fun releaseRemoteOutputForRender(index: Int, clock: () -> Long,
    release: (Int, Long) -> Unit): Pair<Long, Long> {
    val requested = clock()
    release(index, requested)
    return Pair(requested, clock())
}

/** One global owned decoder. The registry retains even an abandoned/Pending
 * close, preventing GC/cancellation from reopening the actual native permit.
 * Same immutable config may recover after retirement with a fresh ownerId/IDR.
 * Android may abandon a SurfaceView consumer despite a retained wrapper; that
 * is a recovery failure, never proof of delivery, editor rendering or photons. */
class RemoteHardwareDecoder private constructor(
    borrowedSurface: Surface, consumerOwner: Any, private val config: DecoderConfig,
    private val onRendered: (ULong, Long, Long, Long) -> Unit,
    private val engineFactory: () -> DecoderEngine,
    private val releaseSurface: (Surface) -> Unit,
) {
    val ownerId: String = UUID.randomUUID().toString()
    private var borrowed: Surface? = borrowedSurface
    private var consumer: Any? = consumerOwner
    private var surface: Surface? = null
    private val mutableState = MutableStateFlow<DecoderStatus>(DecoderStatus.Starting)
    val state: StateFlow<DecoderStatus> = mutableState.asStateFlow()
    private val mutableTiming = MutableStateFlow<DecoderFrameTiming?>(null)
    val lastTiming: StateFlow<DecoderFrameTiming?> = mutableTiming.asStateFlow()
    val lastFailure: String? get() = failure.get()
    private val mutableRenderIdentity = MutableStateFlow<DecoderRenderIdentity?>(null)
    val lastRenderIdentity: StateFlow<DecoderRenderIdentity?> = mutableRenderIdentity.asStateFlow()
    private val closing = AtomicBoolean(false)
    private val codecReleased = AtomicBoolean(false)
    private val released = AtomicBoolean(false)
    private val closeLock = ReentrantLock()
    private var surfaceRetirer: Thread? = null
    private val surfaceReleased = AtomicBoolean(false)
    private val surfaceReleaseFailure = AtomicReference<String?>(null)
    private val ticketLock = Any()
    private val tickets = DecoderTickets(config)
    private val submitted = ArrayBlockingQueue<DecoderFrame>(1)
    private data class Rendered(val pts: Long, val rendered: Long, val callback: Long)
    private val rendered = ArrayBlockingQueue<Rendered>(1)
    private val failure = AtomicReference<String?>(null)
    private val retryRelease = Semaphore(0)
    private val callbacks = HandlerThread("vw-remote-render-callback")
    private val worker = Thread({ runOwned() }, "vw-remote-decoder")
    @Volatile private var callbacksStarted = false

    fun queue(frame: DecoderFrame): DecoderQueueResult {
        synchronized(ticketLock) {
            if (closing.get()) return DecoderQueueResult.Refused("Retiring")
            if (state.value !is DecoderStatus.Ready) return DecoderQueueResult.Refused("NotReady")
            val reason = tickets.admit(frame, System.nanoTime())
            if (reason != null) { recover(reason); return DecoderQueueResult.RecoveryRequired(reason) }
            val owned = checkNotNull(tickets.outstanding)
            if (!submitted.offer(owned)) { recover("QueueLoss"); return DecoderQueueResult.RecoveryRequired("QueueLoss") }
            return DecoderQueueResult.Accepted
        }
    }

    fun requestRecovery(reason: String): Unit = recover(reason.take(128))

    private fun recover(reason: String) {
        mutableTiming.value = null
        if (failure.compareAndSet(null, reason)) mutableState.value = DecoderStatus.RecoveryRequired(reason)
        closing.set(true)
    }

    private fun runOwned() {
        var engine: DecoderEngine? = null
        try {
            if (closing.get()) return
            callbacks.start(); callbacksStarted = true
            val handler = Handler(callbacks.looper)
            handler.postDelayed({
                if (state.value == DecoderStatus.Starting && !closing.get()) recover("StartupDeadline")
            }, START_DEADLINE_MILLIS)
            // flags0 preserves the borrowed holder.surface reference. Parcel
            // duplicates only the producer reference, not consumer ownership.
            val parcel = Parcel.obtain()
            try {
                checkNotNull(borrowed).writeToParcel(parcel, 0)
                parcel.setDataPosition(0)
                surface = Surface.CREATOR.createFromParcel(parcel)
            } finally { parcel.recycle() }
            borrowed = null
            val ownedSurface = checkNotNull(surface)
            check(ownedSurface.isValid) { "InvalidSurface" }
            if (closing.get()) return
            engine = engineFactory()
            val admission = engine.start(config, ownedSurface, handler) { pts, time, callback ->
                if (!closing.get() && !rendered.offer(Rendered(pts, time, callback))) recover("CallbackQueueLoss")
            }
            if (closing.get()) return
            mutableState.value = DecoderStatus.Ready(admission)
            var input: DecoderFrame? = null
            var inputQueued = false
            var outputReleased = false
            while (!closing.get()) {
                if (!ownedSurface.isValid) { recover("SurfaceAbandoned"); break }
                if (input == null) {
                    input = submitted.poll(4, TimeUnit.MILLISECONDS)
                    inputQueued = false; outputReleased = false
                }
                val frame = input ?: continue
                val now = System.nanoTime()
                if (now < frame.enqueuedLocalNanos || now - frame.enqueuedLocalNanos >= FRAME_DEADLINE_NANOS) {
                    recover("FrameDeadline"); break
                }
                if (!inputQueued) inputQueued = engine.queue(frame.annexB, frame.ptsUs)
                if (closing.get()) break
                if (System.nanoTime() - frame.enqueuedLocalNanos >= FRAME_DEADLINE_NANOS) {
                    recover("FrameDeadline"); break
                }
                if (inputQueued && !outputReleased) {
                    when (val output = engine.output()) {
                        DecoderOutput.None -> Unit
                        DecoderOutput.FormatChanged -> {
                            if (engine.outputDimensions() != Pair(config.codedWidth, config.codedHeight)) recover("OutputGeometryChanged")
                        }
                        is DecoderOutput.Buffer -> {
                            if (output.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) engine.discardOutput(output.index)
                            else if (output.ptsUs != frame.ptsUs || output.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) {
                                engine.discardOutput(output.index); recover("UnexpectedOutput")
                            } else {
                                // The timestamp overload uses the phone's local
                                // System.nanoTime domain; host QPC/media PTS is
                                // retained separately for exact frame identity.
                                val (requested, returned) = releaseRemoteOutputForRender(
                                    output.index, System::nanoTime, engine::renderOutput)
                                synchronized(ticketLock) {
                                    if (!tickets.released(output.ptsUs, requested, returned)) recover("OutputIdentity")
                                }
                                outputReleased = true
                            }
                        }
                    }
                }
                val callback = rendered.poll(4, TimeUnit.MILLISECONDS)
                if (callback != null && !closing.get()) {
                    if (System.nanoTime() - frame.enqueuedLocalNanos >= FRAME_DEADLINE_NANOS) {
                        recover("FrameDeadline"); break
                    }
                    val timing = synchronized(ticketLock) {
                        mutableRenderIdentity.value = tickets.renderIdentity(callback.pts, callback.rendered, callback.callback)
                        tickets.rendered(callback.pts, callback.rendered, callback.callback)?.let { matched ->
                            DecoderFrameTiming(ownerId, matched.scope, matched.generation, matched.ticket,
                                matched.frameId, matched.ptsUs, matched.enqueuedLocalNanos,
                                checkNotNull(tickets.releaseRequestedNanos), checkNotNull(tickets.outputReleaseReturnedNanos),
                                callback.rendered, callback.callback)
                        }
                    }
                    if (timing == null) { recover("RenderedIdentity"); break }
                    if (closing.get()) break
                    mutableTiming.value = timing
                    // Callback is timing/identity only. Controller must still
                    // fence ownerId and validate the native retained ticket.
                    onRendered(timing.ticket, callback.pts, callback.rendered, callback.callback)
                    input = null
                }
            }
        } catch (error: Exception) {
            recover("DecoderFailure:${error.javaClass.simpleName}")
        } finally {
            closing.set(true)
            // OS/codec calls can block. Only this retained worker may stop or
            // release them; timeout is an observation, not native retirement.
            var nativeReleased = engine == null
            if (engine != null) {
                try { engine.stop() } catch (_: Exception) { failure.compareAndSet(null, "CodecStopFailed") }
                while (!nativeReleased) {
                    try { engine.release(); nativeReleased = true }
                    catch (_: Exception) {
                        failure.set("CodecReleasePending")
                        mutableState.value = DecoderStatus.RecoveryRequired("CodecReleasePending")
                        // Keep engine, Surface, callback thread and permit. A
                        // subsequent explicit close retries this same owner.
                        retryRelease.acquireUninterruptibly()
                    }
                }
            }
            codecReleased.set(nativeReleased)
            if (callbacksStarted) callbacks.quitSafely()
        }
    }

    /** NonCancellable bounded join observation. Pending blocks controller/link
     * release and replacement. Call again to observe/retry this same owner. */
    suspend fun close(): DecoderRetirement = withContext(NonCancellable + Dispatchers.IO) {
        closing.set(true)
        mutableTiming.value = null
        if (failure.get() == null) mutableState.value = DecoderStatus.Retiring
        retryRelease.release()
        if (!closeLock.tryLock()) return@withContext DecoderRetirement.Pending("RetirementObservationBusy")
        try {
            if (released.get()) return@withContext DecoderRetirement.Retired
            val deadline = System.nanoTime() + FRAME_DEADLINE_NANOS
            try {
                fun join(thread: Thread) {
                    val left = deadline - System.nanoTime()
                    if (left > 0 && thread.isAlive) thread.join(maxOf(1, left / 1_000_000))
                }
                join(worker)
                if (!worker.isAlive && codecReleased.get() && callbacks.isAlive) callbacks.quitSafely()
                if (callbacksStarted) join(callbacks)
                if (worker.isAlive || callbacks.isAlive || !codecReleased.get())
                    return@withContext DecoderRetirement.Pending(failure.get() ?: "CodecOrThreadPending")
                // Surface.release is another potentially blocking native call.
                // Start it only after codec + decode/callback threads retire;
                // retain and join its own worker before returning the permit.
                if (surface == null) surfaceReleased.set(true)
                if (!surfaceReleased.get() && surfaceRetirer?.isAlive != true) {
                    surfaceReleaseFailure.set(null)
                    val ownedSurface = checkNotNull(surface)
                    val task = Thread({
                        try { releaseSurface(ownedSurface); surfaceReleased.set(true) }
                        catch (_: Exception) { surfaceReleaseFailure.set("SurfaceReleasePending") }
                    }, "vw-remote-surface-retire")
                    surfaceRetirer = task
                    try { task.start() }
                    catch (_: Exception) { surfaceReleaseFailure.set("SurfaceRetirementStartFailed") }
                }
                surfaceRetirer?.let { join(it) }
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
                return@withContext DecoderRetirement.Pending("JoinInterrupted")
            } catch (_: Exception) {
                return@withContext DecoderRetirement.Pending("RetirementObservationFailed")
            }
            if (!surfaceReleased.get() || surfaceRetirer?.isAlive == true)
                return@withContext DecoderRetirement.Pending(surfaceReleaseFailure.get() ?: "SurfaceWorkerPending")
            surface = null; borrowed = null; consumer = null
            synchronized(ticketLock) { submitted.clear(); rendered.clear(); tickets.retire() }
            released.set(true); mutableState.value = DecoderStatus.Retired
            retained.compareAndSet(this@RemoteHardwareDecoder, null)
            DecoderRetirement.Retired
        } finally { closeLock.unlock() }
    }

    companion object {
        private val retained = AtomicReference<RemoteHardwareDecoder?>(null)
        fun hasPendingOwner(): Boolean = retained.get() != null
        fun open(surface: Surface, consumerOwner: Any, config: DecoderConfig,
            onRendered: (ULong, Long, Long, Long) -> Unit): DecoderOpenResult =
            openOwned(surface, consumerOwner, config, onRendered) { AndroidDecoderEngine() }

        internal fun openOwned(surface: Surface, consumerOwner: Any, config: DecoderConfig,
            onRendered: (ULong, Long, Long, Long) -> Unit,
            releaseSurface: (Surface) -> Unit = { it.release() },
            engineFactory: () -> DecoderEngine): DecoderOpenResult {
            if (!validDecoderConfig(config)) return DecoderOpenResult.Refused("InvalidConfig")
            if (Build.VERSION.SDK_INT < 30) return DecoderOpenResult.Refused("LowLatencyRequestApiUnavailable")
            val owned = config.copy(vps = config.vps.copyOf(), sps = config.sps.copyOf(), pps = config.pps.copyOf())
            val decoder = RemoteHardwareDecoder(surface, consumerOwner, owned, onRendered, engineFactory, releaseSurface)
            if (!retained.compareAndSet(null, decoder)) return DecoderOpenResult.Refused("PreviousOwnerPending")
            try { decoder.worker.start() }
            catch (_: Exception) {
                decoder.closing.set(true); decoder.codecReleased.set(true)
                decoder.mutableState.value = DecoderStatus.RecoveryRequired("WorkerStartFailed")
                // Caller still receives the retained owner and must close it.
            }
            return DecoderOpenResult.Started(decoder)
        }
    }
}

private class AndroidDecoderEngine : DecoderEngine {
    private var codec: MediaCodec? = null
    private var started = false
    private val info = MediaCodec.BufferInfo()
    override fun start(config: DecoderConfig, surface: Surface, callbacks: Handler,
        onRendered: (Long, Long, Long) -> Unit): DecoderAdmission {
        val mime = MediaFormat.MIMETYPE_VIDEO_HEVC
        val selected = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.firstOrNull { candidate ->
            !candidate.isEncoder && candidate.isHardwareAccelerated && !candidate.isSoftwareOnly &&
                candidate.supportedTypes.any { it.equals(mime, true) } &&
                candidate.getCapabilitiesForType(mime).let { capabilities ->
                    capabilities.profileLevels.any { it.profile == MediaCodecInfo.CodecProfileLevel.HEVCProfileMain } &&
                        capabilities.videoCapabilities?.areSizeAndRateSupported(config.codedWidth, config.codedHeight, 30.0) == true
                }
        } ?: error("HardwareHevcSizeRateUnavailable")
        val native = MediaCodec.createByCodecName(selected.name)
        codec = native // Retained before every fallible configuration call.
        val actual = native.codecInfo
        val capabilities = actual.getCapabilitiesForType(mime)
        check(actual.isHardwareAccelerated && !actual.isSoftwareOnly &&
            capabilities.profileLevels.any { it.profile == MediaCodecInfo.CodecProfileLevel.HEVCProfileMain } &&
            capabilities.videoCapabilities?.areSizeAndRateSupported(config.codedWidth, config.codedHeight, 30.0) == true) {
            "ActualHardwareHevcSizeRateUnavailable"
        }
        val format = MediaFormat.createVideoFormat(mime, config.codedWidth, config.codedHeight)
        format.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
        format.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, MAX_REMOTE_AU)
        format.setFloat(MediaFormat.KEY_OPERATING_RATE, 30f)
        val csd = config.vps + config.sps + config.pps
        format.setByteBuffer("csd-0", ByteBuffer.wrap(csd))
        val vendors = mutableListOf<DecoderVendorCapability>()
        if (Build.VERSION.SDK_INT >= 31) {
            val advertised = native.supportedVendorParameters.take(256).toSet()
            for (key in listOf("vendor.qti-ext-dec-low-latency.enable", "vendor.qti-ext-dec-picture-order.enable")) {
                val type = if (key in advertised) native.getParameterDescriptor(key)?.type else null
                val requested = type == MediaFormat.TYPE_INTEGER
                if (requested) format.setInteger(key, 1)
                vendors += DecoderVendorCapability(key, type, requested, false)
            }
        }
        native.configure(format, surface, null, 0)
        native.setOnFrameRenderedListener({ _, pts, nanos -> onRendered(pts, nanos, System.nanoTime()) }, callbacks)
        native.start(); started = true
        return DecoderAdmission(native.canonicalName, actual.isHardwareAccelerated,
            capabilities.isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency),
            true, true, true, vendors.map { it.copy(configureAccepted = it.requested) })
    }
    override fun queue(bytes: ByteArray, ptsUs: Long): Boolean {
        val native = checkNotNull(codec)
        val index = native.dequeueInputBuffer(0)
        if (index < 0) return false
        val buffer = checkNotNull(native.getInputBuffer(index))
        check(buffer.capacity() >= bytes.size) { "InputCapacity" }
        buffer.clear(); buffer.put(bytes)
        native.queueInputBuffer(index, 0, bytes.size, ptsUs, 0)
        return true
    }
    override fun output(): DecoderOutput = when (val index = checkNotNull(codec).dequeueOutputBuffer(info, 0)) {
        MediaCodec.INFO_TRY_AGAIN_LATER -> DecoderOutput.None
        MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> DecoderOutput.FormatChanged
        MediaCodec.INFO_OUTPUT_BUFFERS_CHANGED -> DecoderOutput.None
        else -> { check(index >= 0); DecoderOutput.Buffer(index, info.presentationTimeUs, info.flags) }
    }
    override fun outputDimensions(): Pair<Int, Int> = checkNotNull(codec).outputFormat.let {
        Pair(it.getInteger(MediaFormat.KEY_WIDTH), it.getInteger(MediaFormat.KEY_HEIGHT))
    }
    override fun discardOutput(index: Int) { checkNotNull(codec).releaseOutputBuffer(index, false) }
    override fun renderOutput(index: Int, requestedLocalNanos: Long) {
        checkNotNull(codec).releaseOutputBuffer(index, requestedLocalNanos)
    }
    override fun stop() { if (started) { checkNotNull(codec).stop(); started = false } }
    override fun release() { codec?.release(); codec = null; started = false }
}
