package com.visualworkbench.shared

import java.nio.file.Files
import org.junit.Assert.*
import org.junit.Test

class PeerLabelStoreTest {
    @Test fun localNameFollowsExactPeerAndCertificateWithoutBecomingAnAddress() {
        val directory = Files.createTempDirectory("vw-peer-label-test-")
        val file = directory.resolve("peer-labels.properties")
        try {
            val store = PeerLabelStore(file)
            assertTrue(store.load().writable)
            assertEquals("Studio PC", store.rename("synthetic-peer", "a".repeat(64), " Studio PC ").nameFor("synthetic-peer", "a".repeat(64)))
            val restored = PeerLabelStore(file).load()
            assertEquals("Studio PC", restored.nameFor("synthetic-peer", "a".repeat(64)))
            assertNull(restored.nameFor("other-peer", "a".repeat(64)))
            assertNull(restored.nameFor("synthetic-peer", "b".repeat(64)))
            assertEquals(1, restored.labels.size)
            assertEquals("😀".repeat(64), checkedPeerLabel("😀".repeat(64)))
            assertNull(checkedPeerLabel("😀".repeat(65)))
            for (invalid in listOf("", " a\nb ", "a\u202Eb", "a\u2066b", "a\uD800b")) assertNull(checkedPeerLabel(invalid))
        } finally { Files.deleteIfExists(file); Files.deleteIfExists(directory) }
    }

    @Test fun unsupportedExistingStorePreservesBytesAndBlocksSaving() {
        val directory = Files.createTempDirectory("vw-peer-label-test-")
        val file = directory.resolve("peer-labels.properties")
        val original = "schema=99\nunknown=preserve\n".toByteArray()
        try {
            Files.write(file, original)
            val store = PeerLabelStore(file)
            assertFalse(store.load().writable)
            assertEquals(PeerLabelIssue.InvalidStore, store.load().issue)
            assertFalse(store.rename("synthetic-peer", "a".repeat(64), "Phone").writable)
            assertArrayEquals(original, Files.readAllBytes(file))
        } finally { Files.deleteIfExists(file); Files.deleteIfExists(directory) }
    }

    @Test fun externallyChangedStoreIsPreservedRatherThanOverwritten() {
        val directory = Files.createTempDirectory("vw-peer-label-test-")
        val file = directory.resolve("peer-labels.properties")
        val changed = "schema=1\n# another writer\n".toByteArray()
        try {
            val store = PeerLabelStore(file)
            assertTrue(store.load().writable)
            Files.write(file, changed)
            val result = store.rename("synthetic-peer", "a".repeat(64), "Phone")
            assertFalse(result.writable)
            assertEquals(PeerLabelIssue.ChangedStore, result.issue)
            assertArrayEquals(changed, Files.readAllBytes(file))
        } finally { Files.deleteIfExists(file); Files.deleteIfExists(directory) }
    }

    @Test fun refreshedAdapterDescriptionDoesNotChangeNumericSelection() {
        val selected = LocalSessionAddress("192.0.2.1", 3u, "Original adapter")
        assertTrue(selected.sameConnection(selected.copy(interfaceName = "Renamed adapter")))
        assertFalse(selected.sameConnection(selected.copy(address = "192.0.2.2")))
        assertFalse(selected.sameConnection(selected.copy(interfaceIndex = 4u)))
    }
}
