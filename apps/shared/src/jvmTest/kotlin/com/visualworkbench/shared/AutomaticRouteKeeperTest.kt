package com.visualworkbench.shared

import java.nio.file.Files
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

/** Addresses here are loopback or documentation ranges only. */
class AutomaticRouteKeeperTest {
    private val phone = PairedDevice("synthetic-phone", "a".repeat(64), 1uL, null)
    private val revoked = PairedDevice("synthetic-old", "b".repeat(64), 1uL, 2uL)
    private val wifi = LocalSessionAddress("192.0.2.10", 2u, "Home network", prefixLength = 24)
    private val usb = LocalSessionAddress("198.51.100.2", 7u, "Phone USB adapter", prefixLength = 24)
    private fun endpoint(carrier: SessionCarrier, address: String) = SessionEndpoint(carrier, address)
    private fun status(carrier: SessionCarrier?, state: SyncStatus) =
        SessionStatus(1uL, state, carrier, 0u, 0u, 0uL, "hash", phone.deviceId, null, null, 0u, null, null, null)

    private class Sessions : WorkbenchSessions {
        val known = HashMap<String, List<SessionEndpoint>>()
        val restores = ArrayList<Pair<String, List<SessionEndpoint>>>()
        var refuse = false
        override suspend fun knownRoutes(peer: String) = known[peer].orEmpty()
        override suspend fun restoreRoutes(peer: String, endpoints: List<SessionEndpoint>) {
            if (refuse) throw SessionFailure(SessionFailureKind.Invalid)
            restores += peer to endpoints; known[peer] = endpoints
        }
        override suspend fun localDevice() = SessionDevice("synthetic-computer", "c".repeat(64))
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
        override suspend fun host(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink = error("unused")
        override suspend fun connect(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink = error("unused")
        override suspend fun receiveProject(path: String, peer: String, endpoints: List<SessionEndpoint>, expectedProjectId: String?, maxBlobBytes: ULong): WorkbenchProject = error("unused")
        override suspend fun close() = Unit
    }
    private class Link : ProjectLink {
        val offers = ArrayList<List<SessionEndpoint>>()
        val listens = ArrayList<List<SessionEndpoint>>()
        var closing = false
        override val endpoints: List<SessionEndpoint> get() = listens.lastOrNull().orEmpty()
        override val changes: Flow<SessionStatus> = emptyFlow()
        override fun status(): SessionStatus = error("unused")
        override fun sendViewport(documentId: String, corners: List<Point>) = Unit
        override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) = Unit
        override fun streamObject(preview: ObjectPreview) = Unit
        override fun streamNewObject(preview: NewObjectPreview) = Unit
        override suspend fun finishPreview(gestureId: String, cancel: Boolean) = Unit
        override suspend fun peerPreviews(documentId: String): PeerPreviews = error("unused")
        override fun offerRoutes(hints: List<SessionEndpoint>) { if (closing) throw SessionFailure(SessionFailureKind.Closed); offers += hints }
        override suspend fun listenOn(bindEndpoints: List<SessionEndpoint>): List<SessionEndpoint> {
            if (closing) throw SessionFailure(SessionFailureKind.Closed)
            listens += bindEndpoints
            return bindEndpoints
        }
        override suspend fun close() = Unit
    }
    private class Discovery : RouteDiscovery {
        val advertised = ArrayList<List<String>>()
        var nearby = emptyList<String>()
        var stops = 0
        override suspend fun advertise(service: WorkbenchSessions, endpoints: List<String>) { advertised += endpoints }
        override suspend fun found(service: WorkbenchSessions, local: List<LocalSessionAddress>) = nearby
        override suspend fun stop() { stops++ }
    }
    private class Fixture(var local: List<LocalSessionAddress>) {
        val directory = Files.createTempDirectory("vw-route-keeper-test-")
        val file = directory.resolve("known-routes.properties")
        val discovery = Discovery()
        val sessions = Sessions()
        fun keeper() = AutomaticRouteKeeper(KnownRouteStore(file), discovery,
            { saved -> { address -> address.interfaceName != null && address.interfaceName == saved.tetherAdapter } },
            { saved -> { address -> address.interfaceName in saved.adapters } }, { local })
        fun close() { Files.deleteIfExists(file); Files.deleteIfExists(directory) }
    }
    private fun fixture(local: List<LocalSessionAddress> = listOf(wifi), block: suspend Fixture.() -> Unit) = runBlocking {
        val fixture = Fixture(local)
        try { fixture.block() } finally { fixture.close() }
    }

    @Test fun savedRoutesReturnToTheCoreOncePerPairedDeviceAndUnpairedOnesAreDropped() = fixture {
        val first = keeper()
        val phoneKey = first.key(phone)!!; val oldKey = first.key(revoked)!!
        val route = endpoint(SessionCarrier.QuicWifi, "192.0.2.20:44242")
        first.change { it.remember(oldKey, listOf(route)).remember(phoneKey, listOf(route)) }

        val restarted = keeper()
        restarted.restore(sessions, listOf(phone, revoked))
        restarted.restore(sessions, listOf(phone, revoked))
        assertEquals(listOf(phone.deviceId to listOf(route)), sessions.restores)
        assertEquals(setOf(phoneKey), restarted.saved().peers)
        // A new service instance (the app restarted its sessions) is supplied again.
        restarted.restore(Sessions().also { it.known.clear() }, listOf(phone))
        assertEquals(1, sessions.restores.size)

        // Routes the core refuses are forgotten rather than offered again.
        val refusing = Sessions().apply { refuse = true }
        keeper().restore(refusing, listOf(phone))
        assertTrue(keeper().saved().routesFor(phoneKey).isEmpty())
        assertNull(first.key(phone.copy(fingerprint = "short")))
    }

    @Test fun aHostKeepsItsListenersMatchedToTheNetworkAndAnnouncesThem() = fixture {
        val keeper = keeper(); val link = Link()
        // Nothing is listened on until the owner has chosen an adapter.
        assertTrue(keeper.plan().binds.isEmpty() && keeper.plan().usable.isEmpty())
        keeper.keep(sessions, link, true, phone, status(null, SyncStatus.Offline), keeper.plan())
        assertTrue(link.listens.isEmpty() && discovery.advertised.isEmpty())
        keeper.change { it.withTetherAdapter("Phone USB adapter").withAdapter("Home network") }
        var plan = keeper.plan()
        assertEquals(listOf(endpoint(SessionCarrier.QuicWifi, "192.0.2.10:44242")), plan.binds)
        assertNull(plan.tetherInterface)
        keeper.keep(sessions, link, true, phone, status(null, SyncStatus.Offline), plan)
        keeper.keep(sessions, link, true, phone, status(null, SyncStatus.Offline), plan)
        assertEquals(1, link.listens.size)
        assertEquals(listOf(listOf("192.0.2.10:44242")), discovery.advertised)

        // The phone's USB tethering comes up: one more listener, no restart.
        local = listOf(wifi, usb)
        plan = keeper.plan()
        assertEquals(7u, plan.tetherInterface)
        assertNotEquals("", plan.network)
        keeper.keep(sessions, link, true, phone, status(SessionCarrier.QuicWifi, SyncStatus.Synced), plan)
        assertEquals(listOf(endpoint(SessionCarrier.QuicTether, "198.51.100.2:44242"), endpoint(SessionCarrier.QuicWifi, "192.0.2.10:44242")), link.listens.last())
        assertEquals(listOf("198.51.100.2:44242", "192.0.2.10:44242"), discovery.advertised.last())
        // A host dials nobody, so it only notes which device it last served.
        assertTrue(keeper.saved().isLastPeer(keeper.key(phone)))
        assertTrue(keeper.saved().routesFor(keeper.key(phone)).isEmpty())

        // The network vanishes for a moment: the listeners are left alone.
        local = emptyList()
        keeper.keep(sessions, link, true, phone, status(null, SyncStatus.Reconnecting), keeper.plan())
        assertEquals(2, link.listens.size)
        // Two adapters with the remembered USB name: neither is labelled USB,
        // and without that label neither was chosen for listening.
        local = listOf(wifi, usb, usb.copy(address = "198.51.100.3", interfaceIndex = 8u))
        assertEquals(listOf(endpoint(SessionCarrier.QuicWifi, "192.0.2.10:44242")), keeper.plan().binds)
        assertNull(keeper.plan().tetherInterface)
        // An adapter the owner never chose (a VPN, a virtual switch) is not used.
        local = listOf(wifi, LocalSessionAddress("203.0.113.5", 11u, "Work tunnel", prefixLength = 24))
        assertEquals(listOf(endpoint(SessionCarrier.QuicWifi, "192.0.2.10:44242")), keeper.plan().binds)
        assertEquals(listOf(wifi), keeper.plan().usable)

        keeper.release()
        assertEquals(1, discovery.stops)
        assertFalse(keeper.plan().toString().contains("192.0.2"))
    }

    @Test fun aClientIsOfferedWhatDiscoveryFindsAndOnlyAnAuthenticatedRouteIsSaved() = fixture(listOf(wifi, usb)) {
        val keeper = keeper(); val link = Link()
        keeper.change { it.withTetherAdapter("Phone USB adapter").withAdapter("Home network") }
        discovery.nearby = listOf("192.0.2.30:44242", "198.51.100.9:44242", "192.0.2.10:44242")
        keeper.keep(sessions, link, false, phone, status(null, SyncStatus.Offline), keeper.plan())
        keeper.keep(sessions, link, false, phone, status(null, SyncStatus.Reconnecting), keeper.plan())
        assertEquals(listOf(listOf(endpoint(SessionCarrier.QuicTether, "198.51.100.9:44242"), endpoint(SessionCarrier.QuicWifi, "192.0.2.30:44242"))), link.offers)
        // Discovery saw addresses; none is saved until the core has authenticated one.
        assertTrue(keeper.saved().peers.isEmpty())
        assertTrue(link.listens.isEmpty())

        sessions.known[phone.deviceId] = listOf(endpoint(SessionCarrier.QuicTether, "198.51.100.9:44242"))
        keeper.keep(sessions, link, false, phone, status(SessionCarrier.QuicTether, SyncStatus.Syncing), keeper.plan())
        assertEquals(sessions.known[phone.deviceId], keeper().saved().routesFor(keeper.key(phone)))
        discovery.nearby = emptyList()
        keeper.keep(sessions, link, false, phone, status(SessionCarrier.QuicTether, SyncStatus.Synced), keeper.plan())
        assertEquals(emptyList<SessionEndpoint>(), link.offers.last())
        // A local link going or coming (USB tethering switched back on) asks the
        // core for a fresh look at once, even with nothing new to offer.
        val asked = link.offers.size
        local = listOf(wifi)
        keeper.keep(sessions, link, false, phone, status(SessionCarrier.QuicWifi, SyncStatus.Synced), keeper.plan())
        local = listOf(wifi, usb)
        keeper.keep(sessions, link, false, phone, status(SessionCarrier.QuicWifi, SyncStatus.Synced), keeper.plan())
        keeper.keep(sessions, link, false, phone, status(SessionCarrier.QuicWifi, SyncStatus.Synced), keeper.plan())
        assertEquals(asked + 2, link.offers.size)
        assertEquals(emptyList<SessionEndpoint>(), link.offers.last())

        // A closing link is not an error and does not stop the next step.
        link.closing = true
        discovery.nearby = listOf("192.0.2.31:44242")
        keeper.keep(sessions, link, false, phone, status(null, SyncStatus.Offline), keeper.plan())
        // Another automatic connection starts from a clean slate.
        val next = Link()
        keeper.keep(sessions, next, false, phone, status(null, SyncStatus.Offline), keeper.plan())
        assertEquals(1, discovery.stops)
        assertEquals(listOf(listOf(endpoint(SessionCarrier.QuicWifi, "192.0.2.31:44242"))), next.offers)
    }

    @Test fun aConnectionMadeByHandIsRememberedOnceItHasAuthenticated() = fixture {
        val keeper = keeper()
        val typed = endpoint(SessionCarrier.QuicWifi, "192.0.2.60:47000")
        sessions.known[phone.deviceId] = listOf(typed)
        // Not connected yet: nothing is saved.
        keeper.authenticated(sessions, phone, null)
        keeper.authenticated(sessions, phone, status(null, SyncStatus.Reconnecting))
        keeper.authenticated(sessions, phone, status(SessionCarrier.QuicWifi, SyncStatus.Offline))
        assertTrue(keeper.saved().peers.isEmpty())
        assertFalse(Files.exists(file))
        keeper.authenticated(sessions, phone, status(SessionCarrier.QuicWifi, SyncStatus.Synced))
        assertEquals(listOf(typed), keeper().saved().routesFor(keeper.key(phone)))
        assertTrue(keeper().saved().isLastPeer(keeper.key(phone)))
        // Nothing was announced or looked for on its account.
        assertTrue(discovery.advertised.isEmpty()); assertEquals(0, discovery.stops)
    }

    @Test fun theAddressFromAScannedPairingCodeIsOnlyAHintUntilItHasAuthenticated() = fixture(listOf(wifi, usb)) {
        val keeper = keeper(); val link = Link()
        keeper.change { it.withTetherAdapter("Phone USB adapter").withAdapter("Home network") }
        val before = String(Files.readAllBytes(file), Charsets.ISO_8859_1)
        // Not an address, or not a paired device: nothing is kept at all.
        keeper.offerFromPairing(phone, "computer.example:44242")
        keeper.offerFromPairing(revoked, "198.51.100.6:44242")
        assertTrue(keeper.hints(sessions, phone.deviceId, keeper.plan(), look = false).isEmpty())
        assertTrue(keeper.hints(sessions, revoked.deviceId, keeper.plan(), look = false).isEmpty())

        keeper.offerFromPairing(phone, " 198.51.100.5:44242 ")
        val hint = endpoint(SessionCarrier.QuicTether, "198.51.100.5:44242")
        // It is offered to the core as a place to try, ahead of what discovery finds...
        assertEquals(listOf(hint), keeper.hints(sessions, phone.deviceId, keeper.plan(), look = false))
        discovery.nearby = listOf("192.0.2.30:44242")
        keeper.keep(sessions, link, false, phone, status(null, SyncStatus.Reconnecting), keeper.plan())
        assertEquals(listOf(hint, endpoint(SessionCarrier.QuicWifi, "192.0.2.30:44242")), link.offers.single())
        // ...and it is neither saved nor handed to the core as a place where the device was authenticated.
        assertTrue(sessions.restores.isEmpty() && sessions.known.isEmpty())
        assertTrue(keeper.saved().peers.isEmpty())
        assertEquals(before, String(Files.readAllBytes(file), Charsets.ISO_8859_1))
        // Another run knows nothing of it.
        assertTrue(keeper().hints(sessions, phone.deviceId, keeper.plan(), look = false).isEmpty())

        // The core authenticated the device there: now it is a route, saved as one, and no longer a hint.
        sessions.known[phone.deviceId] = listOf(hint)
        keeper.keep(sessions, link, false, phone, status(SessionCarrier.QuicTether, SyncStatus.Synced), keeper.plan())
        assertEquals(listOf(hint), keeper().saved().routesFor(keeper.key(phone)))
        assertTrue(keeper.hints(sessions, phone.deviceId, keeper.plan(), look = false).isEmpty())
    }

    @Test fun aDeviceThatIsNoLongerPairedLosesEverythingKeptForIt() = fixture {
        val keeper = keeper()
        val phoneKey = keeper.key(phone)!!
        val route = endpoint(SessionCarrier.QuicWifi, "192.0.2.20:44242")
        keeper.change { it.remember(phoneKey, listOf(route)) }
        keeper.offerFromPairing(phone, "192.0.2.21:44242")
        // Still paired: nothing goes.
        keeper.prune(listOf(phone))
        assertEquals(listOf(route), keeper.saved().routesFor(phoneKey))
        assertEquals(1, keeper.hints(sessions, phone.deviceId, keeper.plan(), look = false).size)
        // Revoked: its routes, the note that it was used last and its pairing hint go,
        // without a session service and whatever the automatic switch says.
        assertFalse(keeper.saved().automatic)
        keeper.prune(listOf(phone.copy(revokedAtMs = 5uL)))
        val after = keeper().saved()
        assertTrue(after.peers.isEmpty()); assertFalse(after.isLastPeer(phoneKey))
        assertFalse(String(Files.readAllBytes(file), Charsets.ISO_8859_1).contains(phoneKey))
        assertTrue(keeper.hints(sessions, phone.deviceId, keeper.plan(), look = false).isEmpty())
        // A key the pairing list has never heard of is dropped the same way.
        keeper.change { it.remember(keeper.key(revoked)!!, listOf(route)) }
        keeper.prune(emptyList())
        assertTrue(keeper().saved().peers.isEmpty())
        assertTrue(sessions.restores.isEmpty())
    }
}
