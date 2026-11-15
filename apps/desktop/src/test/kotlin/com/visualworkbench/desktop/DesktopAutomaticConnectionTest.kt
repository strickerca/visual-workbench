package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import java.nio.file.Files
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

/** Addresses here are loopback or documentation ranges only. */
class DesktopAutomaticConnectionTest {
    private val phone = PairedDevice("peer", "22".repeat(32), 1uL, null)
    private val tablet = PairedDevice("tablet", "44".repeat(32), 2uL, null)
    private val wifi = LocalSessionAddress("192.0.2.10", 2u, "Home network", prefixLength = 24)
    private val usb = LocalSessionAddress("198.51.100.2", 7u, "Phone USB adapter", prefixLength = 24)
    private val hostRole = ProjectSessionRole("project", "installed", "installed", true, 0u, 0u)
    private val replicaRole = ProjectSessionRole("project", "installed", "peer", false, 0u, 0u)

    private class AutomaticLink : LinkFixture() {
        val listens = ArrayList<List<SessionEndpoint>>(); val offers = ArrayList<List<SessionEndpoint>>()
        override val endpoints: List<SessionEndpoint> get() = listens.lastOrNull().orEmpty()
        override suspend fun listenOn(bindEndpoints: List<SessionEndpoint>): List<SessionEndpoint> { listens += bindEndpoints; return bindEndpoints }
        override fun offerRoutes(hints: List<SessionEndpoint>) { offers += hints }
    }
    private class Sessions(var paired: List<PairedDevice>) : SessionFixture() {
        val known = HashMap<String, List<SessionEndpoint>>(); val restored = ArrayList<String>()
        override suspend fun pairedDevices() = paired
        override suspend fun knownRoutes(peer: String) = known[peer].orEmpty()
        override suspend fun restoreRoutes(peer: String, endpoints: List<SessionEndpoint>) { restored += peer; known[peer] = endpoints }
    }
    private class Editor : AutomaticEditor {
        override var project: String? = null
        override var role: ProjectSessionRole? = null
        override var status: SessionStatus? = null
        override var busy = false
        var inspections = 0
        val starts = ArrayList<Pair<String, List<SessionEndpoint>>>()
        var link: AutomaticLink? = null
        var startFails = false
        var tether: (() -> UInt?)? = null
        override fun inspectRole() { inspections++ }
        override fun start(peer: String, endpoints: List<SessionEndpoint>, tether: () -> UInt?, finished: (Exception?) -> Unit): Boolean {
            starts += peer to endpoints; this.tether = tether
            if (startFails) { finished(SessionFailure(SessionFailureKind.Transport)); return true }
            link = AutomaticLink(); status = sessionStatus(1u, SyncStatus.Offline); finished(null)
            return true
        }
        override fun automaticLink(): ProjectLink? = link
        val hints = ArrayList<String>()
        override fun hint(message: String) { hints += message }
        fun disconnect() { link = null; status = null }
    }
    private class Nearby : RouteDiscovery {
        var found = emptyList<String>(); val advertised = ArrayList<List<String>>(); var stops = 0
        override suspend fun advertise(service: WorkbenchSessions, endpoints: List<String>) { advertised += endpoints }
        override suspend fun found(service: WorkbenchSessions, local: List<LocalSessionAddress>) = found
        override suspend fun stop() { stops++ }
    }
    private class Fixture(scope: CoroutineScope, paired: List<PairedDevice>, var local: List<LocalSessionAddress>) {
        val directory = Files.createTempDirectory("vw-desktop-automatic-test-")
        val file = directory.resolve("known-routes.properties")
        val service = Sessions(paired); val editor = Editor(); val nearby = Nearby()
        val sessions = DesktopSessionController(scope, "installed", { service })
        val keeper = AutomaticRouteKeeper(KnownRouteStore(file), nearby, ::chosenTetherAdapter, ::chosenAdapters) { local }
        val automatic = DesktopAutomaticConnection(scope, sessions, editor, keeper)
        suspend fun close() { automatic.close(); sessions.close(); Files.deleteIfExists(file); Files.deleteIfExists(directory) }
    }
    /** [enabled]: the owner switched automatic connection on. [chosen]: the
     * owner has already picked the home network adapter once. */
    private fun fixture(paired: List<PairedDevice> = listOf(phone), local: List<LocalSessionAddress> = listOf(wifi), chosen: Boolean = true,
        enabled: Boolean = true, block: suspend Fixture.() -> Unit) = runBlocking {
        val fixture = Fixture(this, paired, local)
        try {
            fixture.sessions.requireService()
            if (enabled) fixture.keeper.change { it.withAutomatic(true) }
            if (chosen) fixture.keeper.change { it.withAdapter("Home network") }
            fixture.block()
        } finally { fixture.close() }
    }
    private fun connected(carrier: SessionCarrier) = sessionStatus(2u, SyncStatus.Synced, carrier)
    private suspend fun until(condition: () -> Boolean) = withTimeout(5000) { while (!condition()) delay(5) }

    @Test fun anOpenProjectIsSharedWithThePairedPhoneWithoutAnyChoice() = fixture {
        automatic.step()
        assertTrue(editor.starts.isEmpty())
        assertEquals("Open a project and your paired phone connects to it by itself.", automaticSummary(automatic.state.value, false, 1, null))

        // A project opens; its role is checked once, then the listener starts.
        editor.project = "1:project"
        automatic.step(); automatic.step()
        assertEquals(1, editor.inspections); assertTrue(editor.starts.isEmpty())
        editor.role = hostRole
        automatic.step()
        assertEquals(listOf("peer" to listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.10:$AUTOMATIC_SESSION_PORT"))), editor.starts)
        val link = editor.link!!
        // From the next look on the connection is kept listening and announced.
        automatic.step()
        assertEquals(1, link.listens.size)
        assertEquals(listOf(listOf("192.0.2.10:$AUTOMATIC_SESSION_PORT")), nearby.advertised)
        assertTrue(automatic.state.value.active && automatic.state.value.hosting && automatic.state.value.listening == 1)
        assertTrue("Waiting for your phone" in automaticSummary(automatic.state.value, true, 1, editor.status))
        automatic.step()
        assertEquals(1, editor.starts.size); assertEquals(1, link.listens.size)

        // The owner chose the USB adapter once; when it appears it is added as USB.
        automatic.rememberTetherAdapter(usb)
        until { automatic.state.value.tetherAdapter != null }
        local = listOf(wifi, usb)
        editor.status = connected(SessionCarrier.QuicWifi)
        automatic.step()
        assertEquals(SessionCarrier.QuicTether, link.listens.last().first().carrier)
        assertEquals(7u, editor.tether!!())
        assertEquals("Phone USB adapter", automatic.state.value.tetherAdapter)
        assertTrue("over Wi-Fi" in automaticSummary(automatic.state.value, true, 1, editor.status))
        editor.status = connected(SessionCarrier.QuicTether)
        assertTrue("over USB tethering" in automaticSummary(automatic.state.value, true, 1, editor.status))
        // Nothing shown or kept names an address.
        assertFalse(automatic.state.value.toString().contains("192.0.2"))
    }

    @Test fun untilTheOwnerSwitchesItOnItDoesNothingAndKeepsNothing() = fixture(enabled = false, chosen = false) {
        // This is how it ships until it has been run on the real devices.
        assertFalse(AUTOMATIC_BY_DEFAULT); assertFalse(AutomaticStatus().enabled)
        editor.project = "1:project"
        automatic.step(); automatic.step()
        assertEquals(0, editor.inspections)
        editor.role = hostRole
        automatic.step()
        assertTrue(editor.starts.isEmpty() && editor.hints.isEmpty() && service.restored.isEmpty())
        assertTrue(nearby.advertised.isEmpty()); assertEquals(0, nearby.stops)
        assertFalse(automatic.state.value.enabled)
        assertTrue("is off" in automaticSummary(automatic.state.value, true, 1, null))
        // Adapters picked in the connection sheet for pairing or USB are not kept,
        // and a connection made by hand is neither touched nor remembered.
        automatic.rememberAdapter(wifi)?.join(); automatic.rememberTetherAdapter(usb)?.join()
        editor.status = connected(SessionCarrier.QuicWifi)
        service.known["peer"] = listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.40:$AUTOMATIC_SESSION_PORT"))
        automatic.step()
        assertFalse(Files.exists(file))
        assertTrue(automatic.state.value.adapters.isEmpty() && automatic.state.value.tetherAdapter == null)

        // Switched on: a connection made by hand is remembered from then on...
        automatic.setEnabled(true)
        until { automatic.state.value.enabled }
        automatic.step()
        assertTrue(editor.starts.isEmpty())
        assertEquals(service.known["peer"], keeper.saved().routesFor(keeper.key(phone)))
        // ...and with that connection gone and the project opened again, it asks for the one choice it needs.
        editor.status = null; editor.project = "2:project"
        automatic.step()
        assertTrue(editor.starts.isEmpty())
        assertEquals(AutomaticNote.ChooseNetwork, automatic.state.value.note)
        assertEquals(1, editor.hints.size)
    }

    @Test fun itLeavesTheOwnersDecisionsAlone() = fixture {
        editor.project = "1:project"; editor.role = hostRole
        automatic.step(); automatic.step()
        assertEquals(1, editor.starts.size)
        // Disconnect: no silent restart, even when the network changes.
        editor.disconnect()
        automatic.step()
        local = listOf(wifi, usb)
        automatic.step()
        assertEquals(1, editor.starts.size)
        assertEquals(1, nearby.stops)
        assertTrue(automatic.state.value.quiet)
        assertTrue("was closed" in automaticSummary(automatic.state.value, true, 1, null))
        // Switching it off and on again is the owner asking for it.
        automatic.setEnabled(false)
        until { !automatic.state.value.enabled }
        automatic.step()
        assertFalse(automatic.state.value.enabled)
        assertTrue("is off" in automaticSummary(automatic.state.value, true, 1, null))
        automatic.setEnabled(true)
        until { automatic.state.value.enabled }
        automatic.step()
        assertEquals(2, editor.starts.size)
        // A connection made with the manual buttons is not touched or replaced.
        editor.link = null; editor.status = connected(SessionCarrier.QuicWifi)
        automatic.step()
        assertEquals(2, editor.starts.size)
        assertFalse(automatic.state.value.active)
        assertTrue("started by hand" in automaticSummary(automatic.state.value, true, 1, editor.status))
        // While the app is busy or pairing, it waits.
        editor.status = null; editor.project = "2:project"; editor.busy = true
        automatic.step()
        assertEquals(2, editor.starts.size)
        editor.busy = false
        automatic.step()
        assertEquals(3, editor.starts.size)
    }

    @Test fun withoutAnObviousPhoneOrNetworkItSaysSoAndWaits() = fixture(paired = listOf(phone, tablet), local = emptyList()) {
        editor.project = "1:project"; editor.role = hostRole
        automatic.step()
        assertTrue(editor.starts.isEmpty())
        assertEquals(AutomaticNote.ChoosePhone, automatic.state.value.note)
        // The phone used last is the obvious one.
        keeper.change { it.withLastPeer(keeper.key(tablet)!!) }
        automatic.step()
        assertTrue(editor.starts.isEmpty())
        assertEquals(AutomaticNote.NoNetwork, automatic.state.value.note)
        assertTrue("not available right now" in automaticSummary(automatic.state.value, true, 2, null))
        // The network appears: it starts by itself.
        local = listOf(wifi)
        automatic.step()
        assertEquals("tablet", editor.starts.single().first)
        // A failed start is reported quietly and not hammered.
        editor.disconnect(); editor.project = "2:project"; editor.startFails = true
        automatic.step(); automatic.step()
        assertEquals(2, editor.starts.size)
        assertEquals(AutomaticNote.CouldNotStart, automatic.state.value.note)
    }

    @Test fun itListensOnlyOnAdaptersTheOwnerChose() = fixture(local = listOf(wifi, LocalSessionAddress("203.0.113.5", 11u, "Work tunnel", prefixLength = 24)), chosen = false) {
        editor.project = "1:project"; editor.role = hostRole
        automatic.step()
        // Nothing was chosen yet: it asks once instead of listening everywhere.
        assertTrue(editor.starts.isEmpty())
        assertEquals(AutomaticNote.ChooseNetwork, automatic.state.value.note)
        assertTrue("Choose the PC network connection" in automaticSummary(automatic.state.value, true, 1, null))
        // The owner is told once, in the editor, and not nagged.
        local = local + LocalSessionAddress("198.51.100.77", 12u, "Another unchosen adapter", prefixLength = 24)
        automatic.step()
        assertEquals(1, editor.hints.size)
        assertFalse(editor.hints.single().contains("192.0.2"))
        // Loopback is never an adapter to listen on.
        automatic.rememberAdapter(LocalSessionAddress("127.0.0.1", 1u, "Loopback", loopback = true))
        automatic.rememberAdapter(wifi)
        until { automatic.state.value.adapters == listOf("Home network") }
        automatic.step()
        assertEquals(listOf("peer" to listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.10:$AUTOMATIC_SESSION_PORT"))), editor.starts)
        // Forgetting the choice stops new listeners; the live connection is not torn down.
        automatic.forgetAdapters()
        until { automatic.state.value.adapters.isEmpty() }
        automatic.step(); automatic.step()
        assertNotNull(editor.link)
        assertTrue(editor.link!!.listens.isEmpty())
    }

    @Test fun aReplicaDialsItsHostWithHintsFromDiscoveryAndRemembersWhatAuthenticated() = fixture(local = listOf(wifi)) {
        val key = keeper.key(phone)!!
        val saved = SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.40:$AUTOMATIC_SESSION_PORT")
        keeper.change { it.remember(key, listOf(saved)) }
        editor.project = "1:project"; editor.role = replicaRole
        automatic.step()
        // Routes saved by an earlier run went back to the core before dialling.
        assertEquals(listOf("peer"), service.restored)
        assertEquals(listOf("peer" to emptyList<SessionEndpoint>()), editor.starts)
        nearby.found = listOf("192.0.2.50:$AUTOMATIC_SESSION_PORT")
        automatic.step()
        val link = editor.link!!
        assertEquals(listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.50:$AUTOMATIC_SESSION_PORT")), link.offers.last())
        assertTrue(link.listens.isEmpty())
        assertEquals(listOf(saved), keeper.saved().routesFor(key))
        // Only after the core authenticated a route is it saved.
        val proven = SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.50:$AUTOMATIC_SESSION_PORT")
        service.known["peer"] = listOf(proven, saved)
        editor.status = connected(SessionCarrier.QuicWifi)
        automatic.step()
        assertEquals(listOf(proven, saved), keeper.saved().routesFor(key))
        assertTrue(keeper.saved().isLastPeer(key))

        // A replica of a device that is not paired here is never dialled.
        editor.disconnect(); editor.project = "2:project"
        editor.role = replicaRole.copy(hostDeviceId = "stranger")
        automatic.step()
        assertEquals(1, editor.starts.size)
        assertEquals(AutomaticNote.DifferentHost, automatic.state.value.note)
        // Revoking the phone drops what was saved for it.
        service.paired = listOf(phone.copy(revokedAtMs = 9uL))
        sessions.refreshTrust()
        until { sessions.state.value.paired.all { it.revokedAtMs != null } }
        automatic.step()
        assertTrue(keeper.saved().peers.isEmpty())
    }
}
