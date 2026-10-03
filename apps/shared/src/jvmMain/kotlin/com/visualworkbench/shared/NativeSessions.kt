@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import com.visualworkbench.bindings.core.SessionService
import com.visualworkbench.bindings.core.SessionException
import com.visualworkbench.bindings.core.LiveSession
import com.visualworkbench.bindings.core.AppCarrier
import com.visualworkbench.bindings.core.PairingServer as NPairingServer
import com.visualworkbench.bindings.core.PairingConfirmation as NConfirmation
import com.visualworkbench.bindings.core.SessionEndpoint as NEndpoint
import com.visualworkbench.bindings.core.SessionStatus as NStatus
import com.visualworkbench.bindings.core.SyncStatus as NStatusKind
import com.visualworkbench.bindings.core.PreviewKind as NPreviewKind
import com.visualworkbench.bindings.core.ObjectPreview as NObjectPreview
import com.visualworkbench.bindings.core.SampleBatch as NBatch
import com.visualworkbench.bindings.core.Transform as NTransform
import com.visualworkbench.bindings.core.ObjectStyle as NStyle
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.ReceiveProjectOptions
import com.visualworkbench.bindings.core.SessionDiscovery as NDiscovery
import com.visualworkbench.bindings.core.DiscoveryProperty as NDiscoveryProperty
import com.visualworkbench.bindings.core.TetherRouteRisk as NRouteRisk
import com.visualworkbench.bindings.core.NewObjectPreview as NNewObjectPreview
import com.visualworkbench.bindings.core.NewShape as NNewShape
import com.visualworkbench.bindings.core.QueryRect as NRect
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import com.visualworkbench.bindings.core.Cancellation
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal fun sessionFailure(error: SessionException): SessionFailure = SessionFailure(when (error) {
    is SessionException.Invalid -> SessionFailureKind.Invalid
    is SessionException.Backpressure -> SessionFailureKind.Backpressure
    is SessionException.Closed -> SessionFailureKind.Closed
    is SessionException.Cancelled -> SessionFailureKind.Cancelled
    is SessionException.Authentication -> SessionFailureKind.Authentication
    is SessionException.Expired -> SessionFailureKind.Expired
    is SessionException.LockedOut -> SessionFailureKind.LockedOut
    is SessionException.Declined -> SessionFailureKind.Declined
    is SessionException.Storage -> SessionFailureKind.Storage
    is SessionException.Timeout -> SessionFailureKind.Timeout
    is SessionException.Transport -> SessionFailureKind.Transport
    is SessionException.Worker -> SessionFailureKind.Worker
})
private suspend fun <T> sessionCall(block: suspend () -> T): T = withContext(Dispatchers.Default) { try { block() } catch (error: SessionException) { throw sessionFailure(error) } }
private fun <T> sessionNow(block: () -> T): T = try { block() } catch (error: SessionException) { throw sessionFailure(error) }
internal suspend fun sessionFactory(create: suspend () -> SessionService): WorkbenchSessions =
    acquireNative({ try { create() } catch (error: SessionException) { throw sessionFailure(error) } }, { NativeSessions(it) }, { it.destroy() })
internal interface OwnedSession { suspend fun close() }
private suspend fun releaseConfirmation(handle: NConfirmation) { try { handle.decline() } catch (error: SessionException) { if (error !is SessionException.Closed) throw error } finally { handle.destroy() } }
internal suspend fun <T : Any, R : OwnedSession> acquireOwned(owner: SessionOwner, create: suspend (Cancellation) -> T, wrap: (T) -> R, release: suspend (T) -> Unit): R {
    val wrapped = AtomicReference<R?>(null)
    return acquireNative(create, { owner.adopt(wrap(it)).also(wrapped::set) }, { value ->
        val child = wrapped.get()
        if (child != null) child.close() else release(value)
    })
}
internal class SessionOwner {
    private val closed = AtomicBoolean(false)
    private val children = ConcurrentHashMap.newKeySet<OwnedSession>()
    @Synchronized fun <T : OwnedSession> adopt(child: T): T { checkOpen(); children.add(child); return child }
    fun remove(child: OwnedSession) { children.remove(child) }
    fun checkOpen() { if (closed.get()) throw SessionFailure(SessionFailureKind.Closed) }
    suspend fun close() {
        val active = synchronized(this) { if (!closed.compareAndSet(false, true)) return; children.toList() }
        var failed = false
        for (child in active) try { child.close() } catch (_: Exception) { failed = true }
        if (failed) throw SessionFailure(SessionFailureKind.Worker)
    }
}
private class NativeSessions(private val handle: SessionService) : WorkbenchSessions {
    private val owner = SessionOwner()
    private val closed = AtomicBoolean(false)
    private val closeMutex = Mutex()
    override suspend fun localDevice(): SessionDevice = sessionCall { owner.checkOpen(); handle.localDevice().let { SessionDevice(it.deviceId, it.fingerprint) } }
    override suspend fun pairedDevices(): List<PairedDevice> = sessionCall { owner.checkOpen(); handle.pairedDevices().map { PairedDevice(it.deviceId, it.fingerprint, it.pairedAtMs, it.revokedAtMs) } }
    override suspend fun revoke(deviceId: String) { sessionCall { owner.checkOpen(); handle.revokeDevice(deviceId) } }
    override suspend fun projectRole(project: WorkbenchProject): ProjectSessionRole = sessionCall { owner.checkOpen(); handle.projectRole(nativeProjectHandle(project)).let { ProjectSessionRole(it.projectId,it.localDeviceId,it.hostDeviceId,it.isHost,it.pending,it.blocked) } }
    override suspend fun inspectQr(qr: ByteArray): QrDetails = sessionCall { owner.checkOpen(); handle.inspectQr(qr).let { QrDetails(it.peerDeviceId,it.fingerprint,it.expiresAtMs,it.endpoints) } }
    override suspend fun createDiscovery(endpoints: List<String>): SessionDiscovery = acquireOwned(owner, { owner.checkOpen(); handle.createDiscovery(endpoints) }, { NativeDiscovery(it,owner) }, { try { it.closeSession() } finally { it.destroy() } })
    override suspend fun browseDiscovery(localAddresses: List<String>): SessionDiscovery = acquireOwned(owner, { owner.checkOpen(); handle.browseDiscovery(localAddresses) }, { NativeDiscovery(it,owner) }, { try { it.closeSession() } finally { it.destroy() } })
    override suspend fun tetherRouteWarnings(interfaceIndex: UInt): List<TetherRouteWarning> = sessionCall { owner.checkOpen(); handle.tetherRouteWarnings(interfaceIndex).map { TetherRouteWarning(it.family,it.interfaceIndex,when(it.risk) { NRouteRisk.PREFERRED->TetherRouteRisk.Preferred;NRouteRisk.TIED->TetherRouteRisk.Tied;NRouteRisk.ONLY_DEFAULT->TetherRouteRisk.OnlyDefault },it.fixCommand,it.revertCommand) } }
    override suspend fun listenPairing(bindEndpoint: String): PairingServer = acquireOwned(owner, { owner.checkOpen(); handle.listenPairing(bindEndpoint) }, { NativePairingServer(it, owner) }, { try { it.closeSession() } finally { it.destroy() } })
    override suspend fun joinQr(qr: ByteArray, endpoint: String): String = sessionCall { owner.checkOpen(); handle.joinQr(qr, endpoint) }
    override suspend fun joinCode(code: String, endpoint: String): PairingConfirmation = acquireOwned(owner, { owner.checkOpen(); handle.joinCode(code, endpoint, it) }, { NativeConfirmation(it, owner) }, ::releaseConfirmation)
    override suspend fun host(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink = acquireOwned(owner, { owner.checkOpen(); handle.hostProject(nativeProjectHandle(project), peer, endpoints.map { it.native() }) }, { NativeLink(it, owner) }, { try { it.closeSession() } finally { it.destroy() } })
    override suspend fun connect(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink = acquireOwned(owner, { owner.checkOpen(); handle.connectProject(nativeProjectHandle(project), peer, endpoints.map { it.native() }) }, { NativeLink(it, owner) }, { try { it.closeSession() } finally { it.destroy() } })
    override suspend fun receiveProject(path: String, peer: String, endpoints: List<SessionEndpoint>, expectedProjectId: String?, maxBlobBytes: ULong): WorkbenchProject = acquireNative({ owner.checkOpen(); handle.receiveProject(ReceiveProjectOptions(path, peer, endpoints.map { it.native() }, expectedProjectId, maxBlobBytes), it) }, { owner.checkOpen(); wrapNativeProject(it) }, { try { it.closeSession() } finally { it.destroy() } })
    override suspend fun close() { releaseNative { closeMutex.withLock { if (closed.compareAndSet(false, true)) try { owner.close() } finally { handle.destroy() } } } }
}
private class NativeDiscovery(private val handle: NDiscovery, private val owner: SessionOwner) : SessionDiscovery, OwnedSession {
    private val closed=AtomicBoolean(false)
    private val closeMutex=Mutex()
    private fun checkOpen() { owner.checkOpen();if(closed.get()) throw SessionFailure(SessionFailureKind.Closed) }
    override val advertisement: DiscoveryAdvertisement? get()=sessionNow { checkOpen();handle.advertisement()?.let { DiscoveryAdvertisement(it.instance,it.serviceType,it.port,it.properties.map { p->DiscoveryProperty(p.key,p.value) }) } }
    override suspend fun observe(serviceType: String, properties: List<DiscoveryProperty>, endpoints: List<String>) { sessionCall { checkOpen();handle.observe(serviceType,properties.map { NDiscoveryProperty(it.key,it.value) },endpoints) } }
    override suspend fun forget(discoveryId: String) { sessionCall { checkOpen();handle.forget(discoveryId) } }
    override suspend fun snapshot(): List<DiscoveredPeer> = sessionCall { checkOpen();handle.snapshot().map { DiscoveredPeer(it.discoveryId,it.endpoints) } }
    override suspend fun close() { releaseNative { closeMutex.withLock { if(closed.compareAndSet(false,true)) try { handle.closeSession() } finally { handle.destroy();owner.remove(this@NativeDiscovery) } } } }
}
private class NativePairingServer(private val handle: NPairingServer, private val owner: SessionOwner) : PairingServer, OwnedSession {
    private val closed = AtomicBoolean(false)
    private val closeMutex = Mutex()
    override val endpoint: String get() = handle.localEndpoint()
    override suspend fun offer(useCode: Boolean): PairingOffer = sessionCall { handle.offer(useCode).let { PairingOffer(it.qr, it.code, it.expiresAtMs, it.endpoint) } }
    override suspend fun accept(): PairingResult {
        val wrapped = AtomicReference<NativeConfirmation?>(null)
        return acquireNative({ handle.accept(it) }, { outcome ->
        val confirmation = outcome.confirmation
        val peer = outcome.pairedDeviceId
        when { confirmation != null && peer == null -> PairingResult.Confirm(owner.adopt(NativeConfirmation(confirmation, owner)).also(wrapped::set))
            peer != null && confirmation == null -> PairingResult.Paired(peer)
            else -> throw SessionFailure(SessionFailureKind.Invalid) }
        }, { outcome -> wrapped.get()?.close() ?: outcome.confirmation?.let { releaseConfirmation(it) } })
    }
    override suspend fun close() { releaseNative { closeMutex.withLock { if (closed.compareAndSet(false, true)) try { handle.closeSession() } finally { handle.destroy(); owner.remove(this@NativePairingServer) } } } }
}
private class NativeConfirmation(private val handle: NConfirmation, private val owner: SessionOwner) : PairingConfirmation, OwnedSession {
    private val closed = AtomicBoolean(false)
    private val closeMutex = Mutex()
    override val fingerprint: String get() = handle.fingerprint()
    override suspend fun confirm(displayedFingerprint: String): String = try { sessionCall { handle.confirm(displayedFingerprint) } } finally { close() }
    override suspend fun decline() { close() }
    override suspend fun close() { releaseNative { closeMutex.withLock { if (closed.compareAndSet(false, true)) try { handle.decline() } catch (error: SessionException) { if (error !is SessionException.Closed) throw sessionFailure(error) } finally { handle.destroy(); owner.remove(this@NativeConfirmation) } } } }
}
private class NativeLink(private val handle: LiveSession, private val owner: SessionOwner) : ProjectLink, OwnedSession {
    private val closed = AtomicBoolean(false)
    private val closeMutex = Mutex()
    override val endpoints: List<SessionEndpoint> get() = handle.endpoints().map { it.common() }
    override val changes: Flow<SessionStatus> = flow {
        var sequence = 0uL
        while (currentCoroutineContext().isActive && !closed.get()) { val next = sessionCall { handle.waitStatus(sequence) }; if (next.sequence > sequence) { sequence = next.sequence; emit(next.common()) } }
    }
    override fun status(): SessionStatus = sessionNow { handle.status().common() }
    override fun sendViewport(documentId: String, corners: List<Point>) { sessionNow { handle.sendViewport(documentId, corners.map { NPoint(it.x, it.y) }) } }
    override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) { sessionNow { handle.streamStroke(nativeStrokeHandle(stroke), NBatch(batch.sequence, batch.x.toList(), batch.y.toList(), batch.timeMs.toList(), batch.pressure.toList(), batch.tilt.toList(), batch.orientation.toList()), firstSample) } }
    override fun streamObject(preview: ObjectPreview) { sessionNow { val t = preview.transform; val s = preview.style; handle.streamObject(NObjectPreview(preview.gestureId, preview.documentId, preview.objectId, preview.sequence, when (preview.kind) { PreviewKind.HandleDrag -> NPreviewKind.HANDLE_DRAG; PreviewKind.Slider -> NPreviewKind.SLIDER; PreviewKind.Shape -> NPreviewKind.SHAPE }, NTransform(t.a, t.b, t.c, t.d, t.e, t.f), NStyle(s.rgba, s.width, s.screenConstantWidth, s.fill))) } }
    override fun streamNewObject(preview: NewObjectPreview) { sessionNow {
        val t=preview.transform;val s=preview.style
        val shape=when(val v=preview.shape) { is Shape.Line->NNewShape.Line(v.points.map { NPoint(it.x,it.y) });is Shape.Arrow->NNewShape.Arrow(v.points.map { NPoint(it.x,it.y) });is Shape.Rectangle->NNewShape.Rectangle(NRect(v.rectangle.x,v.rectangle.y,v.rectangle.width,v.rectangle.height));is Shape.Ellipse->NNewShape.Ellipse(NRect(v.rectangle.x,v.rectangle.y,v.rectangle.width,v.rectangle.height));is Shape.Text->NNewShape.Text(NPoint(v.anchor.x,v.anchor.y),v.text,v.font,v.size);else->throw SessionFailure(SessionFailureKind.Invalid) }
        handle.streamNewObject(NNewObjectPreview(preview.gestureId,preview.documentId,preview.objectId,preview.layerId,preview.sequence,preview.createdAtMs,shape,NTransform(t.a,t.b,t.c,t.d,t.e,t.f),NStyle(s.rgba,s.width,s.screenConstantWidth,s.fill)))
    } }
    override suspend fun finishPreview(gestureId: String, cancel: Boolean) { sessionCall { handle.finishPreview(gestureId, cancel) } }
    override suspend fun peerPreviews(documentId: String): PeerPreviews = sessionCall { handle.peerPreviews(documentId).let { PeerPreviews(it.sequence, it.gestureIds, it.items.map(::nativeRenderItem)) } }
    override suspend fun close() { releaseNative { closeMutex.withLock { if (closed.compareAndSet(false, true)) try { handle.closeSession() } finally { handle.destroy(); owner.remove(this@NativeLink) } } } }
}
private fun SessionEndpoint.native(): NEndpoint = NEndpoint(when (carrier) { SessionCarrier.QuicTether -> AppCarrier.QUIC_TETHER; SessionCarrier.QuicWifi -> AppCarrier.QUIC_WIFI; SessionCarrier.TcpAdb -> AppCarrier.TCP_ADB }, address)
private fun NEndpoint.common(): SessionEndpoint = SessionEndpoint(carrier.common(), address)
private fun AppCarrier.common(): SessionCarrier = when (this) { AppCarrier.QUIC_TETHER -> SessionCarrier.QuicTether; AppCarrier.QUIC_WIFI -> SessionCarrier.QuicWifi; AppCarrier.TCP_ADB -> SessionCarrier.TcpAdb }
private fun NStatus.common(): SessionStatus = SessionStatus(sequence, when (status) { NStatusKind.RECONNECTING -> SyncStatus.Reconnecting; NStatusKind.SYNCING -> SyncStatus.Syncing; NStatusKind.SYNCED -> SyncStatus.Synced; NStatusKind.OFFLINE -> SyncStatus.Offline }, carrier?.common(), pending, blocked, hostSeq, stateHash, peerDeviceId, failure, peerViewport?.let { PeerViewport(it.documentId, it.corners.map { p -> Point(p.x, p.y) }) }, echoSamples, echoRttP50Ms, echoRttP95Ms, clockOffsetMs)
