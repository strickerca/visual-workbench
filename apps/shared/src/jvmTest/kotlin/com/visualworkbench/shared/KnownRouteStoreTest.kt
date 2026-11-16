package com.visualworkbench.shared

import java.nio.file.Files
import java.nio.file.Path
import org.junit.Assert.*
import org.junit.Test

/** Addresses here are loopback or documentation ranges only. */
class KnownRouteStoreTest {
    private val phone = KnownRouteStore.peerKey("synthetic-phone", "a".repeat(64))!!
    private val tablet = KnownRouteStore.peerKey("synthetic-tablet", "b".repeat(64))!!
    private val usb = SessionEndpoint(SessionCarrier.QuicTether, "198.51.100.7:44242")
    private val wifi = SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.20:44242")

    private fun withFile(block: (Path) -> Unit) {
        val directory = Files.createTempDirectory("vw-known-routes-test-")
        val file = directory.resolve("known-routes.properties")
        try { block(file) } finally {
            Files.list(directory).use { entries -> entries.forEach(Files::deleteIfExists) }
            Files.deleteIfExists(directory)
        }
    }

    @Test fun routesAreFiledUnderTheDeviceAndItsExactCertificate() {
        assertEquals(64, phone.length)
        assertNotEquals(phone, KnownRouteStore.peerKey("synthetic-phone", "c".repeat(64)))
        assertNotEquals(phone, tablet)
        assertFalse(phone.contains("synthetic"))
        for ((device, fingerprint) in listOf("" to "a".repeat(64), "phone" to "a".repeat(63), "phone" to "A".repeat(64), "p".repeat(129) to "a".repeat(64)))
            assertNull(KnownRouteStore.peerKey(device, fingerprint))
    }

    @Test fun rememberedRoutesAndSettingsSurviveARestartAndNothingElseDoes() = withFile { file ->
        val store = KnownRouteStore(file)
        val fresh = store.load()
        // Off until the owner switches it on.
        assertFalse(AUTOMATIC_BY_DEFAULT); assertFalse(fresh.automatic)
        assertNull(fresh.tetherAdapter); assertNull(fresh.lastPeer); assertTrue(fresh.peers.isEmpty())
        assertFalse(Files.exists(file))
        store.update { it.remember(phone, listOf(usb, wifi, wifi)).withTetherAdapter(" Phone USB adapter ") }
        store.update { it.remember(tablet, listOf(wifi)).withAutomatic(true).withAdapter("Home network").withAdapter(" Office network ").withAdapter("Home network") }
        assertTrue(store.durable)

        val restored = KnownRouteStore(file).load()
        assertTrue(restored.automatic)
        assertEquals("Phone USB adapter", restored.tetherAdapter)
        // The adapters the owner chose come back in the order they were chosen.
        assertEquals(listOf("Home network", "Office network"), restored.adapters.toList())
        assertEquals(restored.adapters, restored.withAdapter("a\nb").withAdapter("").adapters)
        assertEquals((3..10).map { "Adapter $it" }, (1..10).fold(SavedRoutes()) { saved, n -> saved.withAdapter("Adapter $n") }.adapters.toList())
        val cleared = restored.withoutAdapters()
        assertTrue(cleared.adapters.isEmpty()); assertNull(cleared.tetherAdapter)
        assertEquals(listOf(usb, wifi), cleared.routesFor(phone))
        assertEquals(listOf(usb, wifi), restored.routesFor(phone))
        assertEquals(listOf(wifi), restored.routesFor(tablet))
        assertTrue(restored.isLastPeer(tablet)); assertFalse(restored.isLastPeer(phone)); assertFalse(restored.isLastPeer(null))
        assertTrue(restored.routesFor("unknown").isEmpty() && restored.routesFor(null).isEmpty())
        // The file names no device and the value never prints an address.
        val text = String(Files.readAllBytes(file), Charsets.ISO_8859_1)
        assertFalse(text.contains("synthetic"))
        assertFalse(restored.toString().contains("198.51.100.7"))
        // An unchanged value is not rewritten.
        val before = Files.getLastModifiedTime(file)
        KnownRouteStore(file).update { it.withAutomatic(true) }
        assertEquals(before, Files.getLastModifiedTime(file))
    }

    @Test fun onlyBoundedNumericAddressesAreKeptNewestFirst() = withFile { file ->
        val store = KnownRouteStore(file)
        val many = (1..9).map { SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.$it:44242") }
        val saved = store.update {
            it.remember(phone, listOf(SessionEndpoint(SessionCarrier.QuicWifi, "phone.example:44242"),
                SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.5")) + many)
        }
        assertEquals(many.take(6), saved.routesFor(phone))
        assertEquals(many.take(6), KnownRouteStore(file).load().routesFor(phone))
        // Not a pairing: nothing is filed.
        assertTrue(store.update { it.remember("not-a-key", listOf(wifi)) }.routesFor("not-a-key").isEmpty())
        // The newest sixty-four devices are kept.
        var crowd = SavedRoutes()
        val keys = (0 until 70).map { KnownRouteStore.peerKey("device-$it", "d".repeat(64))!! }
        for (key in keys) crowd = crowd.remember(key, listOf(wifi))
        assertEquals(64, crowd.peers.size)
        assertFalse(keys.first() in crowd.peers); assertTrue(keys.last() in crowd.peers)
    }

    @Test fun revokedOrUnpairedDevicesLoseTheirRoutes() = withFile { file ->
        val store = KnownRouteStore(file)
        store.update { it.remember(tablet, listOf(wifi)).remember(phone, listOf(usb)) }
        val kept = store.update { it.keepOnly(setOf(tablet)) }
        assertTrue(kept.routesFor(phone).isEmpty())
        assertNull(kept.lastPeer)
        assertEquals(listOf(wifi), KnownRouteStore(file).load().routesFor(tablet))
        assertTrue(store.update { it.remember(tablet, emptyList()) }.peers.isEmpty())
        assertTrue(KnownRouteStore(file).load().peers.isEmpty())
    }

    @Test fun contentThisVersionDidNotWriteIsIgnoredAndReplacedByTheNextSave() = withFile { file ->
        for (foreign in listOf("schema=2\nautomatic=true\n", "schema=1\n", "schema=1\nautomatic=maybe\n",
            "schema=1\nautomatic=true\nroute.$phone.0=wifi phone.example:1\n",
            "schema=1\nautomatic=true\nroute.$phone.9=wifi 192.0.2.1:1\n",
            "schema=1\nautomatic=true\nroute.short.0=wifi 192.0.2.1:1\n",
            "schema=1\nautomatic=true\nroute.$phone.0=radio 192.0.2.1:1\n",
            "schema=1\nautomatic=true\nlast=nobody\n", "schema=1\nautomatic=true\nextra=1\n",
            "schema=1\nautomatic=true\nadapter.9=Home network\n", "schema=1\nautomatic=true\nadapter.0=\n",
            "x".repeat(70_000))) {
            Files.write(file, foreign.toByteArray(Charsets.ISO_8859_1))
            val store = KnownRouteStore(file)
            val loaded = store.load()
            assertTrue(foreign.take(40), !loaded.automatic && loaded.peers.isEmpty() && loaded.tetherAdapter == null)
            store.update { it.remember(phone, listOf(wifi)) }
            assertTrue(store.durable)
            assertEquals(listOf(wifi), KnownRouteStore(file).load().routesFor(phone))
        }
    }

    @Test fun aRedirectedOrUnusablePathIsNeverWrittenThrough() = withFile { file ->
        val elsewhere = file.resolveSibling("elsewhere.properties")
        Files.write(elsewhere, "owner content".toByteArray())
        try { Files.createSymbolicLink(file, elsewhere) } catch (_: Exception) { return@withFile }
        val store = KnownRouteStore(file)
        assertTrue(store.load().peers.isEmpty())
        val inMemory = store.update { it.remember(phone, listOf(wifi)) }
        // The value still works for this run; it is just not saved.
        assertEquals(listOf(wifi), inMemory.routesFor(phone))
        assertFalse(store.durable)
        assertEquals("owner content", String(Files.readAllBytes(elsewhere)))
        assertTrue(Files.isSymbolicLink(file))
    }
}
