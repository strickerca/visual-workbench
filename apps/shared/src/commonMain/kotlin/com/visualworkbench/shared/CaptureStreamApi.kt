package com.visualworkbench.shared

import kotlinx.coroutines.flow.Flow

/** An authenticated peer announcement never authorizes local OS capture. The
 * platform asks explicitly for receiver allocation and the PC separately chooses
 * and approves its exact capture source. Close children before the project link. */

public enum class CaptureStreamStage { AwaitingSource, Live, Freezing, FrozenLossless, FrozenDegraded, Saved, SourceClosed, Closed }
public data class CaptureStreamSignal(public val sequence: ULong, public val connectionEpoch: ULong,
    public val sourceGeneration: ULong, public val frameSequence: ULong, public val stage: CaptureStreamStage,
    public val width: UInt, public val height: UInt, public val retainedBytes: ULong, public val failure: String?,
    public val sourceAvailable: Boolean = stage != CaptureStreamStage.AwaitingSource && stage != CaptureStreamStage.SourceClosed && stage != CaptureStreamStage.Closed,
    public val sourceName: String? = null)
public data class CaptureStreamFrame(public val captureSessionId: String, public val frameId: ULong,
    public val sourceGeneration: ULong, public val width: UInt, public val height: UInt,
    public val hostX: Int, public val hostY: Int, public val dpiScale: Double,
    public val geometryRevision: UInt, public val timestampNs: Long, public val capturedAtMs: Long,
    public val platform: String)
public data class CaptureFreezeReceipt(public val draftId: String, public val frameId: ULong,
    public val captureSessionId: String, public val sourceGeneration: ULong)

public interface WorkbenchCaptureStream {
    public val signals: Flow<CaptureStreamSignal>
    public fun signal(): CaptureStreamSignal
    /** Sole in-flight draw owner. Null means no new draw, not a display receipt.
     * The platform must serialize actual draw/receipt/accepted pen DOWN. */
    public suspend fun beginDraw(): CaptureStreamDraw?
    public suspend fun frozenBackground(): CaptureFrozenBackground?
    /** Accepted physical pen contact only; never hover, prediction or rejected
     * tools/palms. A returned receipt pins the actual acknowledged display. */
    public suspend fun firstContact(draftId: String): CaptureFreezeReceipt?
    /** Only after the real annotation draft has been disposed or committed. */
    public suspend fun resumeAfterDraftDisposed(): String?
    public suspend fun close(): Unit
}
public interface CaptureStreamPixels {
    public val frame: CaptureStreamFrame
    /** Exact opaque RGBA8, at most 256 KiB. Admit the full platform bitmap/GPU
     * copies before requesting rows. No native memory credit covers them. */
    public suspend fun rgbaRows(firstRow: UInt, rowCount: UInt): ByteArray
    public suspend fun close(): Unit
}
public interface CaptureStreamDraw : CaptureStreamPixels {
    /** Exactly once choice, repeat same choice joins. Drop without a receipt
     * seals the stream: it can never guess whether the platform showed pixels. */
    public suspend fun presented(actuallyShown: Boolean): Unit
}
public interface CaptureFrozenBackground : CaptureStreamPixels {
    public val draftId: String
    public val lossless: Boolean
    public val originalAssetId: String?
}

public data class CapturePublicationSource(public val explicitOwnerAction: Boolean,
    public val captureSessionId: String, public val sourceKind: String,
    public val windowHandle: ULong, public val monitorId: String,
    public val hostX: Int, public val hostY: Int, public val width: UInt, public val height: UInt,
    public val dpiScale: Double, public val geometryRevision: UInt, public val observedTimestampNs: Long,
    public val targetFps: UInt, public val interaction: String = "live_annotate", public val platform: String = "windows",
    public val sourceName: String = sourceKind)
public data class CaptureOriginalRequest(public val sequence: ULong, public val draftId: String, public val frame: CaptureStreamFrame)
public data class CapturePublicationSignal(public val sequence: ULong, public val connectionEpoch: ULong,
    public val sourceGeneration: ULong, public val paused: Boolean, public val closed: Boolean,
    public val repairSequence: ULong, public val original: CaptureOriginalRequest?, public val failure: String?)
public interface WorkbenchCapturePublisher {
    public val signals: Flow<CapturePublicationSignal>
    public fun signal(): CapturePublicationSignal
    /** Exact helper bytes; native admits/decodes the envelope. No application
     * protobuf parser, queue or retry copying belongs at this boundary. */
    public suspend fun publishFrame(encoded: ByteArray): Boolean
    public suspend fun provideOriginal(requestSequence: ULong, identity: CaptureImportDescriptor, encoded: ByteArray): Unit
    public suspend fun originalUnavailable(requestSequence: ULong): Unit
    /** Stop and settle the actual OS producer first; this joins the protocol
     * owner's native codec/transfer work, not that independently owned helper. */
    public suspend fun close(): Unit
}
/** A prompt-only request. The computer must approve and draw the exact region
 * locally. It never enumerates sources or captures pixels before that action. */
public data class CaptureRegionSignal(public val sequence:ULong,public val connectionEpoch:ULong,
    public val requestId:String?,public val outgoingRequestId:String?,public val status:String)

/** Estimate only: the carrier clock offset is peer wall time minus local wall
 * time, derived from the existing echo exchange. It is not photon timing. */
public fun captureFrameAgeMs(capturedAtMs:Long,localNowMs:Long,peerMinusLocalOffsetMs:Double?):Double? {
    if(capturedAtMs<0||localNowMs<0||peerMinusLocalOffsetMs==null||!peerMinusLocalOffsetMs.isFinite())return null
    val age=localNowMs.toDouble()+peerMinusLocalOffsetMs-capturedAtMs.toDouble()
    return age.takeIf{it.isFinite()}?.coerceAtLeast(0.0)
}
