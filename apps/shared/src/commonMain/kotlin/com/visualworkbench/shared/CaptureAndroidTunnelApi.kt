package com.visualworkbench.shared

/** Local owner selection only. Paths select an installation and external
 * platform-tools binary; they never approve runtime bytes or licensing. */
public data class TunnelOptions(
    public val explicitOwnerAction: Boolean,
    public val runtimeDirectory: String,
    public val ownerAdbPath: String,
    public val selectedModel: String,
    public val selectedTransportId: UInt,
    public val sessionId: String,
) {
    override fun toString(): String = "TunnelOptions(model=$selectedModel, transportId=$selectedTransportId)"
}

public enum class TunnelFailureKind {
    Invalid, Consent, AmbiguousDevice, UnauthorizedDevice, RuntimeUnavailable, FeedbackLoop,
    Stale, Limit, Cancelled, Timeout, Storage, Platform, RetirementPending, Closed,
}
public class TunnelFailure(public val kind: TunnelFailureKind) : Exception(kind.name)
public enum class TunnelMode { ViewOnly, OwnerControl, RetirementPending, Closed }
public data class TunnelState(public val mode: TunnelMode, public val mirrorRevision: ULong)
/** Display this exact model/transport and request a fresh explicit local owner
 * confirmation. It is neither a permit nor authority from a peer/project. */
public data class TunnelTarget(public val model: String, public val transportId: UInt, public val mirrorRevision: ULong)
public sealed interface TunnelSource {
    public data class Window(public val window: ULong) : TunnelSource
    public data class Monitor(public val monitor: ULong) : TunnelSource
    public data class Region(public val monitor: ULong, public val x: Int, public val y: Int,
        public val width: UInt, public val height: UInt) : TunnelSource
}
public enum class TunnelHierarchy { Present, Missing, Stale }
public data class TunnelElement(public val left: UInt, public val top: UInt, public val right: UInt,
    public val bottom: UInt, public val className: String, public val resourceId: String,
    public val description: String, public val text: String) {
    override fun toString(): String = "TunnelElement(bounds=[$left,$top,$right,$bottom])"
}
/** Capture identity and actual native acquisition timing, not input-delivery,
 * rendered-frame or physical latency evidence. PNG bytes remain native. */
public data class TunnelCaptureInfo(public val captureSessionId: String, public val sourceGeneration: ULong,
    public val frameId: ULong, public val width: UInt, public val height: UInt, public val encodedBytes: ULong,
    public val assetId: String, public val imageStartedMs: ULong, public val imageFinishedMs: ULong,
    public val imageFinishedTimestampNs: ULong, public val hierarchy: TunnelHierarchy,
    public val hierarchyAgeMs: ULong?, public val hierarchyStartedMs: ULong?, public val hierarchyFinishedMs: ULong?)

public interface WorkbenchAndroidTunnel {
    public suspend fun state(): TunnelState
    public suspend fun beginControlConfirmation(): TunnelTarget
    /** Call only from the local handler for the exact displayed target. Native
     * confirmation is one-shot, expiring and bound to the same owner session. */
    public suspend fun confirmControl(target: TunnelTarget): TunnelState
    public suspend fun cancelControlConfirmation(): Unit
    public suspend fun revokeControl(): TunnelState
    public suspend fun capture(): WorkbenchTunnelCapture
    /** Signals the native session. An interrupted operation must settle and
     * close this owner before another session is opened. */
    public fun cancel(): Unit
    /** Joined producer/child retirement. A failed close remains owned for retry. */
    public suspend fun close(): Unit
}

public interface WorkbenchTunnelCapture {
    public fun info(): TunnelCaptureInfo
    public fun elements(): List<TunnelElement>
    /** Imports the capture's private original. sourcePath is ignored; a returned
     * ordinary project owns its durable files independently of this capture. */
    public suspend fun createProject(options: CreateFileProject): WorkbenchProject
    public suspend fun close(): Unit
}

/** Own around the actual host capture and check again before publishing its
 * result. Calls are synchronous and bounded; desktop runs them on its worker.
 * Close joins an entered check and retains a failed native close for retry. */
public interface WorkbenchTunnelCaptureGuard {
    public fun check(): Unit
    public fun close(): Unit
}

/** Native compiled package approval only; no supplied path/report self-admits. */
public expect fun androidTunnelAvailable(): Boolean
public expect suspend fun openAndroidTunnel(options: TunnelOptions): WorkbenchAndroidTunnel
public expect suspend fun retryAndroidTunnelRetirement(): Unit
public expect fun assertAndroidTunnelRetired(): Unit
public expect fun beginAndroidTunnelCaptureGuard(source: TunnelSource): WorkbenchTunnelCaptureGuard
