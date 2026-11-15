package com.visualworkbench.android.session

import com.visualworkbench.shared.*
import java.nio.file.Files
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

/** Addresses here are loopback or documentation ranges only. */
class AutomaticPairingTest {
    private val computer = PairedDevice("synthetic-computer", "a".repeat(64), 1uL, null)
    private val other = PairedDevice("synthetic-other", "b".repeat(64), 2uL, null)
    private val wifi = LocalSessionAddress("192.0.2.10", 2u, "wlan0", prefixLength = 24)
    private val usb = LocalSessionAddress("198.51.100.1", 5u, "rndis0", prefixLength = 24)
    private fun status(carrier: SessionCarrier?, state: SyncStatus, failure: String? = null) =
        SessionStatus(1uL, state, carrier, 0u, 0u, 0uL, "hash", computer.deviceId, failure, null, 0u, null, null, null)

    private class Link : ProjectLink {
        val offers = ArrayList<List<SessionEndpoint>>(); val listens = ArrayList<List<SessionEndpoint>>()
        override val endpoints: List<SessionEndpoint> get() = listens.lastOrNull().orEmpty()
        override val changes: Flow<SessionStatus> = emptyFlow()
        override fun status(): SessionStatus = error("unused")
        override fun sendViewport(documentId: String, corners: List<Point>) = Unit
        override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) = Unit
        override fun streamObject(preview: ObjectPreview) = Unit
        override fun streamNewObject(preview: NewObjectPreview) = Unit
        override suspend fun finishPreview(gestureId: String, cancel: Boolean) = Unit
        override suspend fun peerPreviews(documentId: String): PeerPreviews = error("unused")
        override fun offerRoutes(hints: List<SessionEndpoint>) { offers += hints }
        override suspend fun listenOn(bindEndpoints: List<SessionEndpoint>): List<SessionEndpoint> { listens += bindEndpoints; return bindEndpoints }
        override suspend fun close() = Unit
    }
    private class Project : WorkbenchProject {
        override val changes: Flow<ProjectChange> = emptyFlow()
        override suspend fun info(): ProjectInfo = error("unused")
        override suspend fun document(documentId: String): DocumentSnapshot = error("unused")
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
        override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
        override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
        override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
        override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("unused")
        override suspend fun export(options: ExportOptions): ExportResult = error("unused")
        override suspend fun close() = Unit
    }
    private class Sessions : WorkbenchSessions {
        val calls = ArrayList<String>()
        val known = HashMap<String, List<SessionEndpoint>>()
        var received: Pair<String, List<SessionEndpoint>>? = null
        val link = Link(); val project = Project()
        override suspend fun knownRoutes(peer: String) = known[peer].orEmpty()
        override suspend fun restoreRoutes(peer: String, endpoints: List<SessionEndpoint>) { known[peer] = endpoints }
        override suspend fun hostAutomatic(project: WorkbenchProject, peer: String, bindEndpoints: List<SessionEndpoint>): ProjectLink {
            calls += "hostAutomatic $peer ${bindEndpoints.joinToString { "${it.carrier}@${it.address}" }}"; return link
        }
        override suspend fun connectAutomatic(project: WorkbenchProject, peer: String, hints: List<SessionEndpoint>): ProjectLink { calls += "connectAutomatic $peer ${hints.size}"; return link }
        override suspend fun receiveProjectAutomatic(path: String, peer: String, hints: List<SessionEndpoint>, expectedProjectId: String?, maxBlobBytes: ULong): WorkbenchProject {
            calls += "receiveAutomatic $peer"; received = path to hints; return project
        }
        override suspend fun host(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink { calls += "host $peer ${endpoints.single().address}"; return link }
        override suspend fun connect(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink { calls += "connect $peer ${endpoints.single().address}"; return link }
        override suspend fun receiveProject(path: String, peer: String, endpoints: List<SessionEndpoint>, expectedProjectId: String?, maxBlobBytes: ULong): WorkbenchProject { calls += "receive $peer ${endpoints.single().address}"; return project }
        override suspend fun localDevice() = SessionDevice("synthetic-phone", "c".repeat(64))
        override suspend fun pairedDevices() = emptyList<PairedDevice>()
        override suspend fun revoke(deviceId: String) = Unit
        override suspend fun projectRole(project: WorkbenchProject): ProjectSessionRole = error("unused")
        override suspend fun inspectQr(qr: ByteArray): QrDetails = error("unused")
        override suspend fun createDiscovery(endpoints: List<String>): SessionDiscovery = error("unused")
        override suspend fun browseDiscovery(localAddresses: List<String>): SessionDiscovery = error("unused")
        override suspend fun tetherRouteWarnings(interfaceIndex: UInt) = emptyList<TetherRouteWarning>()
        override suspend fun listenPairing(bindEndpoint: String): PairingServer = error("unused")
        override suspend fun joinQr(qr: ByteArray, endpoint: String): String = error("unused")
        override suspend fun joinCode(code: String, endpoint: String): PairingConfirmation = error("unused")
        override suspend fun close() { calls += "close" }
    }
    private class Nearby : RouteDiscovery {
        var found = emptyList<String>(); var appearsAfter = 0; var looks = 0
        val advertised = ArrayList<List<String>>(); var stops = 0
        override suspend fun advertise(service: WorkbenchSessions, endpoints: List<String>) { advertised += endpoints }
        override suspend fun found(service: WorkbenchSessions, local: List<LocalSessionAddress>): List<String> = if (++looks > appearsAfter) found else emptyList()
        override suspend fun stop() { stops++ }
    }
    private class Fixture(var local: List<LocalSessionAddress>) {
        val directory = Files.createTempDirectory("vw-android-automatic-test-")
        val file = directory.resolve("known-routes.properties")
        val real = Sessions(); val nearby = Nearby()
        var waited = 0L
        val keeper = AutomaticRouteKeeper(KnownRouteStore(file), nearby,
            { _ -> { address -> isAndroidUsbTetherInterface(address.interfaceName) } },
            { _ -> { address -> isAndroidWifiInterface(address.interfaceName) || isAndroidUsbTetherInterface(address.interfaceName) } }, { local })
        val pairing = AutomaticPairing(keeper, nearby) { waited += it }
        val routed = pairing.routed(real)
        var paired = emptyList<PairedDevice>()
        var view = AutomaticView(null, null, false, false)
        suspend fun step(): String? = pairing.step(real, { paired }) { view }
        fun close() { Files.deleteIfExists(file); Files.deleteIfExists(directory) }
    }
    /** [enabled]: the owner switched automatic connection on. */
    private fun fixture(local: List<LocalSessionAddress> = listOf(wifi), enabled: Boolean = true, block: suspend Fixture.() -> Unit) = runBlocking {
        val fixture = Fixture(local)
        try {
            if (enabled) fixture.pairing.setEnabled(true)
            fixture.block()
        } finally { fixture.close() }
    }
    private fun open(project: String = "project:document", status: SessionStatus? = null, busy: Boolean = false, manual: Boolean = false) =
        AutomaticView(project, status, busy, manual)

    @Test fun anOpenProjectConnectsToTheOnePairedComputerByItself() = fixture {
        paired = listOf(computer)
        assertNull(step())
        view = open(busy = true)
        assertNull(step())
        view = open()
        assertEquals(computer.deviceId, step())
        assertNull(step())

        // The editor presses its ordinary Connect with the automatic endpoints.
        assertTrue(pairing.isAutomatic(pairing.endpoints))
        assertFalse(pairing.isAutomatic(listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.20:44242"))))
        val link = routed.connect(real.project, computer.deviceId, pairing.endpoints)
        assertSame(real.link, link)
        assertEquals(listOf("connectAutomatic ${computer.deviceId} 0"), real.calls)

        // While connected it supplies what discovery finds and saves what authenticated.
        nearby.found = listOf("192.0.2.20:44242", "198.51.100.7:44242")
        local = listOf(wifi, usb)
        view = open(status = status(null, SyncStatus.Reconnecting, "no_known_route"))
        assertNull(step())
        assertEquals(listOf(SessionEndpoint(SessionCarrier.QuicTether, "198.51.100.7:44242"), SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.20:44242")), real.link.offers.last())
        assertTrue(keeper.saved().peers.isEmpty())
        real.known[computer.deviceId] = listOf(SessionEndpoint(SessionCarrier.QuicTether, "198.51.100.7:44242"))
        view = open(status = status(SessionCarrier.QuicTether, SyncStatus.Synced))
        assertNull(step())
        assertEquals(real.known[computer.deviceId], keeper.saved().routesFor(keeper.key(computer)))
    }

    @Test fun untilTheOwnerSwitchesItOnItDoesNothingAndKeepsNothing() = fixture(enabled = false) {
        // This is how it ships until it has been run on the real devices.
        assertFalse(AUTOMATIC_BY_DEFAULT); assertFalse(pairing.enabled)
        paired = listOf(computer)
        view = open()
        assertNull(step()); assertFalse(pairing.load())
        // The address from a scanned pairing code is not kept, nor is a
        // connection made with a typed address.
        pairing.learned(real, computer, "192.0.2.20:$AUTOMATIC_SESSION_PORT")
        assertTrue(real.known.isEmpty())
        real.known[computer.deviceId] = listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.20:47000"))
        view = open(status = status(SessionCarrier.QuicWifi, SyncStatus.Synced))
        assertNull(step())
        assertFalse(Files.exists(file))
        assertEquals(0, nearby.looks); assertEquals(0, nearby.stops); assertTrue(nearby.advertised.isEmpty())
        assertTrue(real.calls.isEmpty())

        // Switched on: the typed connection is remembered from then on, and
        // the next project connects by itself.
        pairing.setEnabled(true)
        assertNull(step())
        assertEquals(real.known[computer.deviceId], keeper.saved().routesFor(keeper.key(computer)))
        view = open(project = "other:document")
        assertEquals(computer.deviceId, step())
    }

    @Test fun theOwnersChoicesAlwaysWin() = fixture {
        paired = listOf(computer)
        view = open()
        assertEquals(computer.deviceId, step())
        routed.connect(real.project, computer.deviceId, pairing.endpoints)
        // Disconnected before the next step looked: still the owner's decision.
        assertNull(step())
        assertTrue(pairing.quiet)
        local = listOf(wifi, usb)
        assertNull(step())
        assertEquals(1, nearby.stops)
        // The project is opened again: a fresh start.
        pairing.projectClosed()
        assertEquals(computer.deviceId, step())
        // The editor had become busy and did not connect: ask again.
        pairing.missed()
        assertEquals(computer.deviceId, step())
        // Switched off: nothing starts. Switched on again: it does.
        pairing.projectClosed()
        pairing.setEnabled(false)
        assertNull(step()); assertFalse(pairing.enabled)
        pairing.setEnabled(true)
        assertEquals(computer.deviceId, step())
        // A typed address means the manual flow is in charge.
        pairing.projectClosed()
        view = open(manual = true)
        assertNull(step())
        // Typed endpoints go to the ordinary calls unchanged.
        val typed = listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.20:47000"))
        routed.connect(real.project, computer.deviceId, typed)
        routed.host(real.project, computer.deviceId, typed)
        routed.receiveProject("/unused", computer.deviceId, typed, null, 1uL)
        assertEquals(listOf("connect ${computer.deviceId} 192.0.2.20:47000", "host ${computer.deviceId} 192.0.2.20:47000", "receive ${computer.deviceId} 192.0.2.20:47000"),
            real.calls.takeLast(3))
        // A connection made by hand is never one that switching off would end.
        assertFalse(pairing.waiting(status(null, SyncStatus.Offline)))
        routed.close()
        assertEquals("close", real.calls.last())
    }

    @Test fun itWaitsWhenThereIsNoNetworkOrNoObviousComputer() = fixture(local = emptyList()) {
        paired = listOf(computer)
        view = open()
        // No network: nothing is attempted, and nothing is used up.
        assertNull(step())
        local = listOf(wifi)
        assertEquals(computer.deviceId, step())
        // Hosting with no address to listen on is refused, not faked.
        local = emptyList()
        val failure = runCatching { routed.host(real.project, computer.deviceId, pairing.endpoints) }.exceptionOrNull()
        assertEquals(SessionFailureKind.Transport, (failure as SessionFailure).kind)
        // Two paired computers and no history: the owner chooses.
        pairing.projectClosed()
        local = listOf(wifi)
        paired = listOf(computer, other)
        assertNull(step())
        keeper.change { it.withLastPeer(keeper.key(other)!!) }
        assertEquals(other.deviceId, step())
        // A revoked computer is never chosen.
        pairing.projectClosed()
        paired = listOf(computer.copy(revokedAtMs = 5uL), other.copy(revokedAtMs = 6uL))
        assertNull(step())
    }

    @Test fun aPhoneOwnedProjectListensOnItsOwnLinksWithUsbFirst() = fixture(listOf(wifi, usb,
        // Mobile data is never listened on.
        LocalSessionAddress("203.0.113.9", 9u, "rmnet_data0", prefixLength = 24))) {
        paired = listOf(computer)
        view = open()
        assertEquals(computer.deviceId, step())
        routed.host(real.project, computer.deviceId, pairing.endpoints)
        assertEquals("hostAutomatic ${computer.deviceId} QuicTether@198.51.100.1:$AUTOMATIC_SESSION_PORT, QuicWifi@192.0.2.10:$AUTOMATIC_SESSION_PORT", real.calls.single())
        // Until the computer connects it is only waiting, which switching off would end.
        assertTrue(pairing.waiting(status(null, SyncStatus.Offline)))
        assertFalse(pairing.waiting(status(SessionCarrier.QuicTether, SyncStatus.Synced)))
        assertFalse(pairing.waiting(null))
        view = open(status = status(null, SyncStatus.Offline))
        assertNull(step())
        assertEquals(2, real.link.listens.single().size)
        assertEquals(listOf(listOf("198.51.100.1:$AUTOMATIC_SESSION_PORT", "192.0.2.10:$AUTOMATIC_SESSION_PORT")), nearby.advertised)
        // In the background it stops announcing; the connection itself stays.
        pairing.rest()
        assertEquals(1, nearby.stops)
    }

    @Test fun receivingWaitsBrieflyForDiscoveryWhenNothingIsRemembered() = fixture {
        nearby.found = listOf("192.0.2.20:44242"); nearby.appearsAfter = 3
        routed.receiveProject("/synthetic/path", computer.deviceId, pairing.endpoints, null, 1uL)
        assertEquals(listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.20:44242")), real.received!!.second)
        assertEquals(750L, waited)
        // A remembered route needs no wait at all.
        waited = 0; nearby.looks = 0; nearby.appearsAfter = 100
        real.known[computer.deviceId] = listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.30:44242"))
        routed.receiveProject("/synthetic/path", computer.deviceId, pairing.endpoints, null, 1uL)
        assertTrue(real.received!!.second.isEmpty()); assertEquals(0L, waited)
        // Nothing found and nothing remembered: it gives up after a bounded wait.
        real.known.clear(); nearby.looks = 0
        routed.receiveProject("/synthetic/path", computer.deviceId, pairing.endpoints, null, 1uL)
        assertTrue(real.received!!.second.isEmpty()); assertEquals(4000L, waited)
    }

    @Test fun theConnectScreenSaysWhatIsHappeningInPlainWords() {
        fun line(automatic: Boolean = true, quiet: Boolean = false, manual: Boolean = false, paired: Boolean = true, status: SessionStatus? = null) =
            automaticConnectionLine(automatic, quiet, manual, paired, status)
        assertTrue("is off" in line(automatic = false))
        assertTrue("manual" in line(manual = true))
        assertTrue("Pair a computer once" in line(paired = false))
        assertTrue("over USB tethering" in line(status = status(SessionCarrier.QuicTether, SyncStatus.Synced)))
        assertTrue("over Wi-Fi" in line(status = status(SessionCarrier.QuicWifi, SyncStatus.Syncing)))
        assertTrue("not been found yet" in line(status = status(null, SyncStatus.Offline, "no_known_route")))
        assertTrue("Looking for your computer" in line(status = status(null, SyncStatus.Reconnecting)))
        assertTrue("Disconnected" in line(quiet = true))
        assertTrue("No address is needed" in line())
        // A status never claims a connection the core has not authenticated.
        assertFalse("Connected" in line(status = status(null, SyncStatus.Synced)))
    }
}
