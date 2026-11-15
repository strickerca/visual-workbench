package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/** Addresses here are loopback or documentation ranges only. */
class AutomaticRoutesTest {
    private val wifi = LocalSessionAddress("192.0.2.10", 2u, "wlan0", prefixLength = 24)
    private val usb = LocalSessionAddress("198.51.100.1", 5u, "rndis0", prefixLength = 24)
    private val loopback = LocalSessionAddress("127.0.0.1", 1u, "lo", loopback = true, prefixLength = 8)
    private val local = listOf(loopback, wifi, usb)
    private fun phoneTether(address: LocalSessionAddress) = isAndroidUsbTetherInterface(address.interfaceName)
    private fun device(id: String, revoked: Boolean = false) =
        PairedDevice(id, "ab".repeat(32), 1uL, if (revoked) 2uL else null)

    @Test fun onlyThePhonesOwnUsbInterfaceNamesCountAsTethering() {
        for (name in listOf("rndis0", "usb0", "ncm0", "rndis12")) assertTrue(isAndroidUsbTetherInterface(name), name)
        for (name in listOf(null, "", "wlan0", "rndis", "usb", "rndis0x", "xrndis0", "usb123", "USB0", "ncm-0", "eth0"))
            assertFalse(isAndroidUsbTetherInterface(name), name.toString())
        // Wi-Fi, including the phone's own hotspot, but never mobile data or anything else.
        for (name in listOf("wlan0", "wlan1", "swlan0", "softap0", "ap_br_wlan2")) assertTrue(isAndroidWifiInterface(name), name)
        for (name in listOf(null, "", "wlan", "rmnet_data0", "rmnet0", "ccmni1", "tun0", "lo", "rndis0", "wlan0x", "p2p-wlan0-0", "dummy0", "eth0"))
            assertFalse(isAndroidWifiInterface(name), name.toString())
    }

    @Test fun hintsAreLabelledByTheLocalLinkTheyAreOnAndNeverIncludeThisDevice() {
        val hints = classifyRouteHints(
            listOf(
                "192.0.2.20:44242",        // on the Wi-Fi subnet
                " 198.51.100.7:44242 ",    // on the USB tethering subnet
                "203.0.113.9:44242",       // on no local subnet
                "[2001:db8::7]:44242",
                "192.0.2.20:44242",        // repeated
                "192.0.2.10:44242",        // this device
                "127.0.0.1:44242", "[::1]:44242",
                "printer.example:44242", "192.0.2.20", "192.0.2.20:0", "192.0.2.999:1", "192.0.2.20:70000", "",
            ),
            local, ::phoneTether,
        )
        assertEquals(
            listOf(
                SessionEndpoint(SessionCarrier.QuicTether, "198.51.100.7:44242"),
                SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.20:44242"),
                SessionEndpoint(SessionCarrier.QuicWifi, "203.0.113.9:44242"),
                SessionEndpoint(SessionCarrier.QuicWifi, "[2001:db8::7]:44242"),
            ),
            hints,
        )
        // Without a known subnet size nothing is ever labelled USB.
        val unsized = local.map { it.copy(prefixLength = null) }
        assertTrue(classifyRouteHints(listOf("198.51.100.7:44242"), unsized, ::phoneTether).all { it.carrier == SessionCarrier.QuicWifi })
        // An adapter that merely looks similar is not USB unless the caller says so.
        assertTrue(classifyRouteHints(listOf("198.51.100.7:44242"), local) { false }.all { it.carrier == SessionCarrier.QuicWifi })
        assertEquals(16, classifyRouteHints((1..40).map { "192.0.2.20:$it" }, local, ::phoneTether).size)
        assertTrue(isNumericSessionEndpoint("[fe80::1%7]:8") && !isNumericSessionEndpoint("[fe80::1%7]") && !isNumericSessionEndpoint("a:1"))
    }

    @Test fun automaticListenersUseRealIpv4LinksOnlyWithUsbFirst() {
        val binds = automaticBindEndpoints(
            local + listOf(
                LocalSessionAddress("2001:db8::10", 2u, "wlan0", prefixLength = 64),
                LocalSessionAddress("169.254.7.7", 9u, "unconfigured", prefixLength = 16),
                LocalSessionAddress("192.0.2.10", 2u, "wlan0", prefixLength = 24),
            ),
            ::phoneTether,
        )
        assertEquals(
            listOf(
                SessionEndpoint(SessionCarrier.QuicTether, "198.51.100.1:$AUTOMATIC_SESSION_PORT"),
                SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.10:$AUTOMATIC_SESSION_PORT"),
            ),
            binds,
        )
        assertTrue(automaticBindEndpoints(listOf(loopback), ::phoneTether).isEmpty())
        val many = (1..20).map { LocalSessionAddress("192.0.2.$it", it.toUInt(), "adapter $it", prefixLength = 24) }
        assertEquals(6, automaticBindEndpoints(many, ::phoneTether).size)
        assertEquals("192.0.2.1:47000", automaticBindEndpoints(many, ::phoneTether, 47000).first().address)
    }

    @Test fun thePeerIsChosenOnlyWhenTheChoiceIsObvious() {
        val phone = device("phone"); val tablet = device("tablet"); val old = device("old", revoked = true)
        val host = ProjectSessionRole("project", "pc", "pc", true, 0u, 0u)
        val replica = ProjectSessionRole("project", "phone", "pc", false, 0u, 0u)
        assertEquals(phone, automaticPeer(listOf(phone, old), host) { false })
        assertEquals(phone, automaticPeer(listOf(phone), null) { false })
        assertNull(automaticPeer(listOf(phone, tablet), host) { false })
        assertEquals(tablet, automaticPeer(listOf(phone, tablet), host) { it.deviceId == "tablet" })
        assertNull(automaticPeer(listOf(old), host) { true })
        assertNull(automaticPeer(emptyList(), host) { true })
        // A replica belongs to one host; no other paired device will do.
        val pc = device("pc")
        assertEquals(pc, automaticPeer(listOf(pc, tablet), replica) { it.deviceId == "tablet" })
        assertNull(automaticPeer(listOf(tablet), replica) { true })
        assertNull(automaticPeer(listOf(device("pc", revoked = true)), replica) { true })
    }

    private fun situation(project: String? = "p1", peer: String? = "phone", linked: Boolean = false, busy: Boolean = false,
        enabled: Boolean = true, network: String = "n1") = AutomaticSituation(enabled, project, peer, linked, busy, network)

    @Test fun itStartsOncePerProjectPeerAndNetworkAndOnlyWhenAllowed() {
        val auto = AutomaticConnection()
        assertNull(auto.next(situation(project = null)))
        assertNull(auto.next(situation(enabled = false)))
        assertNull(auto.next(situation(busy = true)))
        assertNull(auto.next(situation(peer = null)))
        assertEquals("phone", auto.next(situation()))
        // The attempt may have failed (nothing to listen on); it is not repeated blindly.
        assertNull(auto.next(situation()))
        // A change in the local network is a reason to try again.
        assertEquals("phone", auto.next(situation(network = "n2")))
        assertNull(auto.next(situation(network = "n2")))
        // Another project or another paired device is a new decision.
        assertEquals("phone", auto.next(situation(project = "p2")))
        assertEquals("tablet", auto.next(situation(project = "p2", peer = "tablet")))
        assertFalse(situation().toString().contains("phone"))
    }

    @Test fun itNeverWorksAgainstTheOwner() {
        val auto = AutomaticConnection()
        assertEquals("phone", auto.next(situation()))
        assertNull(auto.next(situation(linked = true)))
        // The connection is gone: the owner disconnected, connected by hand or revoked.
        assertNull(auto.next(situation()))
        assertNull(auto.next(situation(network = "n2")))
        // Asking again, or opening the project again, lets it start once more.
        auto.resume()
        assertEquals("phone", auto.next(situation(network = "n2")))
        assertNull(auto.next(situation(linked = true, network = "n2")))
        assertNull(auto.next(situation(network = "n2")))
        assertNull(auto.next(situation(project = null)))
        assertEquals("phone", auto.next(situation(network = "n2")))
        // A link that belonged to another project does not silence this one.
        assertNull(auto.next(situation(project = "p3", linked = true)))
        assertEquals("phone", auto.next(situation(project = "p4")))
        assertFalse(auto.isQuietFor("p4")); assertFalse(auto.isQuietFor(null))
    }

    @Test fun aConnectionThatGoesBeforeTheNextLookStillCountsAsTheOwnersChoice() {
        val auto = AutomaticConnection()
        assertEquals("phone", auto.next(situation()))
        // It was created and disconnected again between two looks.
        auto.started("p1")
        assertNull(auto.next(situation(network = "n2")))
        assertTrue(auto.isQuietFor("p1"))
        // The same project closed and opened again between two looks is a fresh start.
        auto.closed()
        assertFalse(auto.isQuietFor("p1"))
        assertEquals("phone", auto.next(situation(network = "n2")))
    }
}
