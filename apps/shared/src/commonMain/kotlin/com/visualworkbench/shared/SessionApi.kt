package com.visualworkbench.shared

import kotlinx.coroutines.flow.Flow

public interface WorkbenchSessions {
    public suspend fun localDevice(): SessionDevice
    public suspend fun pairedDevices(): List<PairedDevice>
    public suspend fun revoke(deviceId: String)
    public suspend fun projectRole(project: WorkbenchProject): ProjectSessionRole
    public suspend fun inspectQr(qr: ByteArray): QrDetails
    public suspend fun createDiscovery(endpoints: List<String>): SessionDiscovery
    public suspend fun browseDiscovery(localAddresses: List<String> = emptyList()): SessionDiscovery
    /** Windows only. Read-only snapshot for the explicitly selected interface;
     * callers refresh it for every tether connect/reconnect. No commands run. */
    public suspend fun tetherRouteWarnings(interfaceIndex: UInt): List<TetherRouteWarning>
    public suspend fun listenPairing(bindEndpoint: String): PairingServer
    public suspend fun joinQr(qr: ByteArray, endpoint: String): String
    public suspend fun joinCode(code: String, endpoint: String): PairingConfirmation
    public suspend fun host(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink
    public suspend fun connect(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink
    public suspend fun receiveProject(path: String, peer: String, endpoints: List<SessionEndpoint>, expectedProjectId: String? = null, maxBlobBytes: ULong = 2uL * 1024uL * 1024uL * 1024uL): WorkbenchProject
    public suspend fun close()
}
public data class SessionDevice(public val deviceId: String, public val fingerprint: String)
public data class ProjectSessionRole(public val projectId: String, public val localDeviceId: String, public val hostDeviceId: String, public val isHost: Boolean, public val pending: UInt, public val blocked: UInt)
public data class QrDetails(public val peerDeviceId: String, public val fingerprint: String, public val expiresAtMs: ULong, public val endpoints: List<String>)
public data class DiscoveryProperty(public val key: String, public val value: String)
public data class DiscoveryAdvertisement(public val instance: String, public val serviceType: String, public val port: UShort, public val properties: List<DiscoveryProperty>)
/** Untrusted numeric address hints. Never use discoveryId as a paired DeviceId. */
public data class DiscoveredPeer(public val discoveryId: String, public val endpoints: List<String>)
public interface SessionDiscovery {
    /** Null for browse-only operation; never advertise an absent listener. */
    public val advertisement: DiscoveryAdvertisement?
    public suspend fun observe(serviceType: String, properties: List<DiscoveryProperty>, endpoints: List<String>)
    public suspend fun forget(discoveryId: String)
    public suspend fun snapshot(): List<DiscoveredPeer>
    public suspend fun close()
}
public enum class TetherRouteRisk { Preferred, Tied, OnlyDefault }
public data class TetherRouteWarning(public val family: String, public val interfaceIndex: UInt, public val risk: TetherRouteRisk, public val fixCommand: String?, public val revertCommand: String?)
public data class LocalSessionAddress(public val address: String, public val interfaceIndex: UInt)
/** Lists candidates only. The user selects the interface, port, and carrier. */
public expect suspend fun localSessionAddresses(): List<LocalSessionAddress>
public expect fun encodePairingQr(qr: ByteArray): String
public expect fun decodePairingQr(text: String): ByteArray
public data class PairedDevice(public val deviceId: String, public val fingerprint: String, public val pairedAtMs: ULong, public val revokedAtMs: ULong?)
public enum class SessionFailureKind { Invalid, Backpressure, Closed, Cancelled, Authentication, Expired, LockedOut, Declined, Storage, Timeout, Transport, Worker }
public class SessionFailure(public val kind: SessionFailureKind) : Exception(kind.name)
public enum class SessionCarrier { QuicTether, QuicWifi, TcpAdb }
public data class SessionEndpoint(public val carrier: SessionCarrier, public val address: String)
/** Secret-bearing UI value: never log it or persist it in project preferences. */
public class PairingOffer(public val qr: ByteArray, public val code: String, public val expiresAtMs: ULong, public val endpoint: String) {
    public fun clear() { qr.fill(0) }
    override fun toString(): String = "PairingOffer([REDACTED])"
}
public sealed interface PairingResult {
    public data class Paired(public val deviceId: String) : PairingResult
    public class Confirm(public val confirmation: PairingConfirmation) : PairingResult
}
public interface PairingServer {
    public val endpoint: String
    public suspend fun offer(useCode: Boolean = false): PairingOffer
    public suspend fun accept(): PairingResult
    public suspend fun close()
}
public interface PairingConfirmation {
    public val fingerprint: String
    /** Invoke only after explicit comparison on both devices. */
    public suspend fun confirm(displayedFingerprint: String): String
    public suspend fun decline()
    public suspend fun close()
}
public enum class SyncStatus { Reconnecting, Syncing, Synced, Offline }
public data class PeerViewport(public val documentId: String, public val corners: List<Point>)
public data class SessionStatus(public val sequence: ULong, public val status: SyncStatus, public val carrier: SessionCarrier?, public val pending: UInt, public val blocked: UInt, public val hostSeq: ULong, public val stateHash: String, public val peerDeviceId: String, public val failure: String?, public val peerViewport: PeerViewport?, public val echoSamples: UInt, public val echoRttP50Ms: Double?, public val echoRttP95Ms: Double?, public val clockOffsetMs: Double?)
public enum class PreviewKind { HandleDrag, Slider, Shape }
public data class ObjectPreview(public val gestureId: String, public val documentId: String, public val objectId: String, public val sequence: UInt, public val kind: PreviewKind, public val transform: Transform, public val style: ObjectStyle)
/** Shape kind, layer, identity and creation time stay fixed within the gesture.
 * Geometry may evolve within that kind; bind final Create to the same gestureId.
 * Large text/polyline payloads may exceed the preview datagram budget. */
public data class NewObjectPreview(public val gestureId: String, public val documentId: String, public val objectId: String, public val layerId: String, public val sequence: UInt, public val createdAtMs: Long, public val shape: Shape, public val transform: Transform, public val style: ObjectStyle)
public data class PeerPreviews(public val sequence: ULong, public val gestureIds: List<String>, public val items: List<RenderItem>)
public interface ProjectLink {
    public val endpoints: List<SessionEndpoint>
    public val changes: Flow<SessionStatus>
    public fun status(): SessionStatus
    public fun sendViewport(documentId: String, corners: List<Point>)
    public fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt)
    public fun streamObject(preview: ObjectPreview)
    public fun streamNewObject(preview: NewObjectPreview)
    /** Requires protocol minor 2 preview lifecycle. Gesture IDs are single-use.
     * cancel=true sends reliable cancellation; false closes further preview
     * admission after the real transaction has been queued durably. A remote
     * completion is not proof of an accepted edit; the visual swaps on accepted
     * OPS or bounded expiry. Generation counters and ordering stay native. */
    public suspend fun finishPreview(gestureId: String, cancel: Boolean)
    public suspend fun peerPreviews(documentId: String): PeerPreviews
    public suspend fun close()
}
