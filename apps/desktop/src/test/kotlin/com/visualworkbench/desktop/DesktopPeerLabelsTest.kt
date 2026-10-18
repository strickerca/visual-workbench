package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import java.nio.file.Files
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test

class DesktopPeerLabelsTest {
    @Test fun invalidLabelFileDoesNotBlockProtectedSessionReadiness() = runBlocking {
        val directory = Files.createTempDirectory("vw-desktop-peer-label-test-")
        val file = directory.resolve("peer-labels.properties")
        val original = "schema=99\n".toByteArray()
        Files.write(file, original)
        val service = SessionFixture()
        val controller = DesktopSessionController(this, "installed", { service }, labelStore = PeerLabelStore(file))
        try {
            assertSame(service, controller.requireService())
            val state = withTimeout(5_000) { controller.state.first { it.labels != null } }
            assertTrue(state.ready)
            assertEquals("installed", state.local?.deviceId)
            assertEquals(listOf("peer"), state.paired.map { it.deviceId })
            assertFalse(checkNotNull(state.labels).writable)
            assertNull(state.message)
            assertArrayEquals(original, Files.readAllBytes(file))
        } finally { controller.close(); Files.deleteIfExists(file); Files.deleteIfExists(directory) }
    }

    @Test fun renameRequiresTheCurrentPeerAndCompleteCertificate() = runBlocking {
        val directory = Files.createTempDirectory("vw-desktop-peer-label-test-")
        val file = directory.resolve("peer-labels.properties")
        val service = SessionFixture()
        val controller = DesktopSessionController(this, "installed", { service }, labelStore = PeerLabelStore(file))
        try {
            controller.requireService()
            withTimeout(5_000) { controller.state.first { it.labels?.writable == true } }
            controller.renamePeer("peer", "33".repeat(32), "Wrong certificate")
            controller.renamePeer("another-peer", "22".repeat(32), "Wrong peer")
            yield()
            assertFalse(Files.exists(file))
            controller.renamePeer("peer", "22".repeat(32), "Studio phone")
            val state = withTimeout(5_000) { controller.state.first {
                it.labels?.nameFor("peer", "22".repeat(32)) == "Studio phone" && !it.labelSaving } }
            assertTrue(state.ready)
            assertEquals(service.pairedDevices(), state.paired)
            assertEquals("Studio phone", PeerLabelStore(file).load().nameFor("peer", "22".repeat(32)))
            assertNull(state.labels?.nameFor("peer", "33".repeat(32)))
        } finally { controller.close(); Files.deleteIfExists(file); Files.deleteIfExists(directory) }
    }
}
