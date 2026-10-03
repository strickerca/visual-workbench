package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test

class DesktopSessionsTest {
    @Test fun installedIdentityIsPassedOnceAndMismatchClosesTheReturnedService() = runBlocking {
        val service = SessionFixture().apply { device = "different-installation" }
        val calls = mutableListOf<String>()
        val controller = DesktopSessionController(this, "installed", { id -> calls += id; service })
        try {
            runCatching { controller.requireService() }.onSuccess { fail("identity mismatch accepted") }
            assertEquals(listOf("installed"), calls)
            assertEquals(1, service.closed)
            assertFalse(controller.state.value.ready)
        } finally { controller.close() }
        assertEquals(1, service.closed)
    }

    @Test fun codeConfirmationRequiresFullDisplayedFingerprintAndDeliberateComparison() = runBlocking {
        val service = SessionFixture()
        val confirmation = ConfirmationFixture(service.order)
        service.server.result.complete(PairingResult.Confirm(confirmation))
        val controller = DesktopSessionController(this, "installed", { service })
        try {
            controller.requireService()
            controller.offer("127.0.0.1:44242", true)
            withTimeout(2000) { controller.state.first { it.fingerprint != null } }
            assertEquals(0, service.server.closed)
            controller.confirmFingerprint(confirmation.fingerprint.take(8), true)
            controller.confirmFingerprint(confirmation.fingerprint, false)
            yield(); assertEquals(0, confirmation.confirmed)
            controller.confirmFingerprint(confirmation.fingerprint, true)
            withTimeout(2000) { controller.state.first { !it.pairing } }
            assertEquals(1, confirmation.confirmed)
            assertEquals(listOf("confirm", "confirmation-close", "discovery-close", "server-close"), service.order)
            assertNull(controller.state.value.fingerprint)
            assertNull(controller.state.value.display)
        } finally { controller.close() }
    }

    @Test fun dismissClearsQrAndSettlesListenerAndDiscoveryBeforeService() = runBlocking {
        val service = SessionFixture()
        val controller = DesktopSessionController(this, "installed", { service })
        controller.requireService()
        controller.offer("127.0.0.1:44242", false)
        val display = withTimeout(2000) { controller.state.first { it.display != null }.display!! }
        assertFalse(display.toString().contains(display.qrText!!))
        controller.cancelPairing()
        assertTrue(display.offer.qr.all { it == 0.toByte() })
        controller.close()
        assertEquals(listOf("discovery-close", "server-close", "service-close"), service.order)
        assertNull(controller.state.value.display)
    }

    @Test fun expiredOfferClosesWithoutAccepting() = runBlocking {
        val service = SessionFixture().apply { server.expires = 1000uL }
        val controller = DesktopSessionController(this, "installed", { service }, { 1001 })
        try {
            controller.requireService(); controller.offer("127.0.0.1:44242", false)
            withTimeout(2000) { controller.state.first { !it.pairing && it.message != null } }
            assertEquals(0, service.server.accepts); assertEquals(1, service.server.closed)
        } finally { controller.close() }
    }

    @Test fun carrierOrderingAndStatusWordsHaveNoImplicitEndpoint() {
        assertTrue(EndpointFields().endpoints().isEmpty())
        val fields = EndpointFields(" 127.0.0.1:1 ", "127.0.0.1:2", "127.0.0.1:3")
        assertEquals(SessionCarrier.entries, fields.endpoints().map { it.carrier })
        assertEquals("127.0.0.1:1", fields.endpoints().first().address)
        assertEquals("OFFLINE (7 pending)", syncLabel(null, 7u))
        for ((status, label) in listOf(SyncStatus.Synced to "SYNCED", SyncStatus.Syncing to "SYNCING", SyncStatus.Reconnecting to "RECONNECTING", SyncStatus.Offline to "OFFLINE (3 pending)"))
            assertEquals(label, syncLabel(sessionStatus(status = status)))
    }

    @Test fun viewportIsIndependentByDefaultAndExplicitMatchFitsTheWholeRectangle() {
        val local = Camera(Point(900.0, 400.0), 2.0, .3, 300.0, 200.0)
        val viewport = PeerViewport("document", listOf(Point(10.0,20.0),Point(110.0,20.0),Point(110.0,70.0),Point(10.0,70.0)))
        val peer = checkNotNull(peerCamera(viewport, local))
        var view = ViewState(local)
        repeat(100) { view = view.receivePeerEdit().receivePeerView(peer) }
        assertEquals(local, view.camera)
        assertFalse(view.followPeer); assertFalse(view.showPeerOutline)
        assertEquals(Point(60.0,45.0), view.matchPeer().camera.center)
        assertEquals(3.0, view.matchPeer().camera.scale, 0.0)
        assertNull(peerCamera(viewport.copy(corners = emptyList()), local))
    }

    @Test fun delayedPreviewCannotPublishAfterLinkCloseAndTetherCheckRunsOnReconnect() = runBlocking {
        val first = LinkFixture().apply { initialStatus = sessionStatus(0u, SyncStatus.Syncing, SessionCarrier.QuicWifi) }
        val captured = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        first.preview = { captured.complete(Unit); withContext(NonCancellable) { release.await() }; PeerPreviews(1u, listOf("gesture"), emptyList()) }
        val binding = RenderBinding(1, "project", "document", 1u, "hash")
        val published = mutableListOf<PeerFrame?>(); var checks = 0
        val owner = DesktopLiveLink(this, { binding }, {}, published::add, { fail(it) })
        owner.install(first) { checks++ }
        captured.await()
        val closing = launch { owner.close() }
        yield(); release.complete(Unit); closing.join()
        assertTrue(published.all { it == null }); assertEquals(1, first.closed)
        val next = LinkFixture()
        owner.install(next) { checks++ }
        try {
            withTimeout(2000) { next.events.subscriptionCount.first { it > 0 } }
            next.events.emit(sessionStatus(1u, SyncStatus.Syncing, SessionCarrier.QuicTether))
            next.events.emit(sessionStatus(2u, SyncStatus.Synced, SessionCarrier.QuicTether))
            next.events.emit(sessionStatus(3u, SyncStatus.Reconnecting, null))
            next.events.emit(sessionStatus(4u, SyncStatus.Syncing, SessionCarrier.QuicTether))
            withTimeout(2000) { while (checks < 2) yield() }
            assertEquals(2, checks)
        } finally { owner.close() }
    }

    @Test fun delayedPreviewCannotReappearAfterOfflineAndSameSequenceCanRefreshOnReconnect() = runBlocking {
        delayedPreviewAcrossStatus(SyncStatus.Offline)
    }

    @Test fun delayedPreviewCannotReappearAfterReconnectingAndSameSequenceCanRefreshOnReconnect() = runBlocking {
        delayedPreviewAcrossStatus(SyncStatus.Reconnecting)
    }
}

private suspend fun CoroutineScope.delayedPreviewAcrossStatus(disconnected: SyncStatus) {
    val link = LinkFixture().apply { initialStatus = sessionStatus(1u, SyncStatus.Synced, SessionCarrier.QuicWifi) }
    val captured = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>(); val delivered = CompletableDeferred<Unit>()
    val observed = CompletableDeferred<Unit>(); var queries = 0
    link.preview = {
        if (++queries == 2) {
            captured.complete(Unit); release.await(); delivered.complete(Unit)
            PeerPreviews(8u, listOf("stale"), emptyList())
        } else PeerPreviews(7u, listOf("current"), emptyList())
    }
    val published = mutableListOf<PeerFrame?>()
    val owner = DesktopLiveLink(this, { RenderBinding(1, "project", "document", 1u, "hash") },
        { if (it?.status == disconnected) observed.complete(Unit) }, published::add, { fail(it) })
    try {
        owner.install(link) { }
        withTimeout(2000) { captured.await(); link.events.subscriptionCount.first { it > 0 } }
        assertEquals(listOf(7uL), published.filterNotNull().map { it.previews.sequence })
        link.events.emit(sessionStatus(2u, disconnected, null))
        withTimeout(2000) { observed.await() }
        assertNull(published.last())
        release.complete(Unit)
        withTimeout(2000) { delivered.await() }; yield()
        assertNull(published.last())
        assertEquals(listOf(7uL), published.filterNotNull().map { it.previews.sequence })
        link.events.emit(sessionStatus(3u, SyncStatus.Synced, SessionCarrier.QuicWifi))
        withTimeout(2000) { while (published.filterNotNull().size < 2) yield() }
        assertEquals(listOf(7uL, 7uL), published.filterNotNull().map { it.previews.sequence })
    } finally { release.complete(Unit); owner.close() }
}

internal open class SessionFixture : WorkbenchSessions {
    var device = "installed"; var closed = 0
    val order = mutableListOf<String>()
    val server = ServerFixture(order)
    var link = LinkFixture()
    override suspend fun localDevice() = SessionDevice(device, "11".repeat(32))
    override suspend fun pairedDevices() = listOf(PairedDevice("peer", "22".repeat(32), 1u, null))
    override suspend fun revoke(deviceId: String) = Unit
    override suspend fun projectRole(project: WorkbenchProject): ProjectSessionRole = project.info().let { ProjectSessionRole(it.projectId, device, device, true, 0u, 0u) }
    override suspend fun inspectQr(qr: ByteArray): QrDetails = error("unused")
    override suspend fun createDiscovery(endpoints: List<String>): SessionDiscovery = DiscoveryFixture(order)
    override suspend fun browseDiscovery(localAddresses: List<String>): SessionDiscovery = DiscoveryFixture(order)
    override suspend fun tetherRouteWarnings(interfaceIndex: UInt) = emptyList<TetherRouteWarning>()
    override suspend fun listenPairing(bindEndpoint: String): PairingServer = server
    override suspend fun joinQr(qr: ByteArray, endpoint: String): String = error("unused")
    override suspend fun joinCode(code: String, endpoint: String): PairingConfirmation = error("unused")
    override suspend fun host(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink = link
    override suspend fun connect(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink = link
    override suspend fun receiveProject(path: String, peer: String, endpoints: List<SessionEndpoint>, expectedProjectId: String?, maxBlobBytes: ULong): WorkbenchProject = error("unused")
    override suspend fun close() { closed++; order += "service-close" }
}
internal class ServerFixture(private val order: MutableList<String>) : PairingServer {
    override val endpoint = "127.0.0.1:44242"
    val result = CompletableDeferred<PairingResult>(); var accepts = 0; var closed = 0
    var expires = (System.currentTimeMillis() + 300_000).toULong()
    override suspend fun offer(useCode: Boolean) = PairingOffer(if(useCode)byteArrayOf()else byteArrayOf(1,2,3), if(useCode)"00000000"else"", expires, endpoint)
    override suspend fun accept(): PairingResult { accepts++; return result.await() }
    override suspend fun close() { closed++; order += "server-close" }
}
internal class ConfirmationFixture(private val order: MutableList<String>) : PairingConfirmation {
    override val fingerprint = "33".repeat(32)
    var confirmed = 0; private var closed = false
    override suspend fun confirm(displayedFingerprint: String): String { assertEquals(fingerprint, displayedFingerprint); confirmed++; order += "confirm"; return "peer" }
    override suspend fun decline() = Unit
    override suspend fun close() { if (!closed) { closed = true; order += "confirmation-close" } }
}
private class DiscoveryFixture(private val order: MutableList<String>) : SessionDiscovery {
    override val advertisement: DiscoveryAdvertisement? = null
    override suspend fun observe(serviceType: String, properties: List<DiscoveryProperty>, endpoints: List<String>) = Unit
    override suspend fun forget(discoveryId: String) = Unit
    override suspend fun snapshot() = emptyList<DiscoveredPeer>()
    override suspend fun close() { order += "discovery-close" }
}
internal open class LinkFixture : ProjectLink {
    val events = MutableSharedFlow<SessionStatus>(extraBufferCapacity = 16)
    override val changes = events
    override val endpoints = listOf(SessionEndpoint(SessionCarrier.QuicWifi,"127.0.0.1:44242"))
    var closed = 0
    val order = mutableListOf<String>()
    val streamed = mutableListOf<SampleBatch>()
    val objects = mutableListOf<ObjectPreview>()
    val newObjects = mutableListOf<NewObjectPreview>()
    val finished = mutableListOf<Pair<String,Boolean>>()
    var preview: suspend () -> PeerPreviews = { PeerPreviews(0u,emptyList(),emptyList()) }
    var initialStatus = sessionStatus()
    override fun status() = initialStatus
    override fun sendViewport(documentId: String, corners: List<Point>) = Unit
    override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) { streamed += batch; order += "stream" }
    override fun streamObject(preview: ObjectPreview) { objects += preview }
    override fun streamNewObject(preview: NewObjectPreview) { newObjects += preview }
    override suspend fun finishPreview(gestureId: String, cancel: Boolean) { finished += gestureId to cancel; order += "finish:$cancel" }
    override suspend fun peerPreviews(documentId: String) = preview()
    override suspend fun close() { closed++; order += "link-close" }
}
internal fun sessionStatus(sequence: ULong = 0u, status: SyncStatus = SyncStatus.Offline, carrier: SessionCarrier? = null) =
    SessionStatus(sequence,status,carrier,3u,0u,7u,"hash","peer",null,null,0u,null,null,null)
