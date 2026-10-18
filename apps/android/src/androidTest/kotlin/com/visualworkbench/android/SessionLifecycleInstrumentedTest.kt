package com.visualworkbench.android

import android.content.ContextWrapper
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.session.PairingController
import com.visualworkbench.android.session.RefreshAdmission
import com.visualworkbench.android.session.validEndpoint
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Synthetic adapters only: never contacts a peer, camera, or trust store. */
@RunWith(AndroidJUnit4::class)
class SessionLifecycleInstrumentedTest {
    @Test fun refreshFenceRejectsOutOfOrderCompletionAndPriorProjectEvents() {
        val gate = RefreshAdmission()
        gate.reset(); assertTrue(gate.observe(1uL))
        val earlier = gate.capture()
        assertTrue(gate.observe(2uL)); val newer = gate.capture()
        assertFalse(gate.observe(1uL)); assertFalse(gate.observe(2uL))
        assertFalse(gate.accepts(earlier)); assertTrue(gate.accepts(newer))
        gate.reset(); assertFalse(gate.accepts(newer)); assertTrue(gate.observe(1uL))
    }

    @Test fun coalescedRefreshDoesNotPublishSameBaseOlderOptimisticState() = runBlocking {
        val gate = RefreshAdmission(); gate.reset()
        val firstRead = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var shown = "initial"
        gate.observe(1uL)
        val slow = launch {
            val captured = gate.capture(); firstRead.complete(Unit); release.await()
            if (gate.accepts(captured)) shown = "old optimistic state at host sequence 4"
        }
        firstRead.await(); gate.observe(2uL)
        val current = gate.capture()
        if (gate.accepts(current)) shown = "new optimistic state at host sequence 4"
        release.complete(Unit); slow.join()
        assertEquals("new optimistic state at host sequence 4", shown)
    }

    @Test fun localEndpointsAreBoundedAndConfiguredCarrierPriorityIsStable() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val controller = withContext(Dispatchers.Main) { PairingController(context(), scope, { "synthetic-device" }, {}) { _, _ -> FakeSessions() } }
        try {
            withContext(Dispatchers.Main) {
                controller.carrier = SessionCarrier.TcpAdb; controller.sessionAddress = "127.0.0.1:43102"
                controller.carrier = SessionCarrier.QuicWifi; controller.sessionAddress = "192.0.2.2:43101"
                controller.carrier = SessionCarrier.QuicTether; controller.sessionAddress = "192.0.2.3:43100"
                assertEquals(listOf(SessionCarrier.QuicTether, SessionCarrier.QuicWifi, SessionCarrier.TcpAdb), controller.endpoint().map { it.carrier })
                assertFalse(validEndpoint("example.invalid:80")); assertFalse(validEndpoint("127.0.0.1:0"))
                assertFalse(validEndpoint("999.0.0.1:65536")); assertFalse(validEndpoint("127.0.0.1:443\ncommand"))
                assertTrue(validEndpoint("[fd00::1]:443"))
            }
        } finally { withContext(Dispatchers.Main) { controller.close(); scope.cancel() } }
    }

    @Test fun backgroundDuringQrInspectionWipesBytesAndRejectsLateUiResult() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var nativeBytes: ByteArray? = null
        val service = object : FakeSessions() {
            override suspend fun inspectQr(qr: ByteArray): QrDetails {
                nativeBytes = qr; entered.complete(Unit)
                withContext(NonCancellable) { release.await() }
                return QrDetails("synthetic-peer", "0".repeat(64), (System.currentTimeMillis() + 100_000).toULong(), listOf("192.0.2.1:443"))
            }
        }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val controller = withContext(Dispatchers.Main) { PairingController(context(), scope, { "synthetic-device" }, {}) { _, _ -> service } }
        val scanned = byteArrayOf(1, 2, 3, 4)
        try {
            withContext(Dispatchers.Main) { controller.scan(scanned) }
            withTimeout(5000) { entered.await() }
            assertArrayEquals(ByteArray(4), scanned)
            withContext(Dispatchers.Main) { controller.leave() }
            assertArrayEquals(ByteArray(4), nativeBytes)
            release.complete(Unit)
            withTimeout(5000) { while (withContext(Dispatchers.Main) { controller.working }) delay(10) }
            withContext(Dispatchers.Main) { assertNull(controller.details); assertNull(controller.fingerprint); assertEquals("", controller.code) }
        } finally { release.complete(Unit); withContext(Dispatchers.Main) { controller.close(); scope.cancel() } }
    }

    @Test fun lateServiceAcquisitionClosesHandleAndPreservesInstallationId() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val service = FakeSessions()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        var supplied: String? = null
        val controller = withContext(Dispatchers.Main) {
            PairingController(context(), scope, { "existing-installation-id" }, {}) { _, id ->
                supplied = id; entered.complete(Unit); withContext(NonCancellable) { release.await() }; service
            }
        }
        try {
            val pending = async(Dispatchers.Main) { runCatching { controller.service() } }
            entered.await()
            val closing = async(Dispatchers.Main) { controller.close() }
            yield(); release.complete(Unit)
            withTimeout(5000) { pending.await(); closing.await() }
            assertEquals("existing-installation-id", supplied); assertEquals(1, service.closes)
        } finally { release.complete(Unit); scope.cancel() }
    }

    @Test fun codeRequiresExplicitFingerprintConfirmationAndBackgroundDeclosesPending() = runBlocking {
        val confirmation = object : PairingConfirmation {
            override val fingerprint = "a".repeat(64)
            var confirmations = 0; var closes = 0
            override suspend fun confirm(displayedFingerprint: String): String { assertEquals(fingerprint, displayedFingerprint); confirmations++; return "synthetic-peer" }
            override suspend fun decline() = Unit
            override suspend fun close() { closes++ }
        }
        val service = object : FakeSessions() { override suspend fun joinCode(code: String, endpoint: String): PairingConfirmation { assertEquals("12345678", code); return confirmation } }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val controller = withContext(Dispatchers.Main) { PairingController(context(), scope, { "synthetic-device" }, {}) { _, _ -> service } }
        try {
            withContext(Dispatchers.Main) { controller.code = "12345678"; controller.pairingEndpoint = "192.0.2.1:443"; controller.pairCode() }
            withTimeout(5000) { while (withContext(Dispatchers.Main) { controller.working }) delay(10) }
            assertEquals(0, confirmation.confirmations)
            withContext(Dispatchers.Main) { assertEquals(confirmation.fingerprint, controller.fingerprint); assertEquals("", controller.code); controller.leave() }
            withTimeout(5000) { while (confirmation.closes == 0) delay(10) }
            assertEquals(0, confirmation.confirmations)
            withContext(Dispatchers.Main) { assertNull(controller.fingerprint) }
        } finally { withContext(Dispatchers.Main) { controller.close(); scope.cancel() } }
    }

    @Test fun loadedTrustedPeersDoNotStartScannerOrReplaceChosenRoutesOnEntry() = runBlocking {
        val peer = PairedDevice("synthetic-peer", "a".repeat(64), 1uL, null)
        val service = object : FakeSessions() { override suspend fun pairedDevices() = listOf(peer) }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val controller = withContext(Dispatchers.Main) { PairingController(context(), scope, { "synthetic-device" }, {}) { _, _ -> service } }
        try {
            withContext(Dispatchers.Main) {
                assertEquals(SessionCarrier.QuicTether, controller.carrier)
                assertFalse(controller.peersLoaded); assertFalse(controller.scanner)
                controller.carrier = SessionCarrier.QuicWifi; controller.sessionAddress = "192.0.2.2:43101"
                controller.enter()
            }
            withTimeout(5000) { while (withContext(Dispatchers.Main) { controller.working }) delay(10) }
            withContext(Dispatchers.Main) {
                assertTrue(controller.peersLoaded); assertEquals(listOf(peer), controller.peers)
                assertFalse(controller.scanner); assertNull(controller.selectedPeer)
                assertEquals(SessionCarrier.QuicWifi, controller.carrier)
                assertEquals("192.0.2.2:43101", controller.sessionAddress)
                assertEquals(listOf(SessionCarrier.QuicWifi), controller.configuredCarriers())
            }
        } finally { withContext(Dispatchers.Main) { controller.close(); scope.cancel() } }
    }

    @Test fun backgroundDuringPeerRefreshRejectsLateKnownEmptyOrLoadedResult() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val service = object : FakeSessions() {
            override suspend fun pairedDevices(): List<PairedDevice> {
                entered.complete(Unit)
                withContext(NonCancellable) { release.await() }
                return listOf(PairedDevice("synthetic-peer", "a".repeat(64), 1uL, null))
            }
        }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val controller = withContext(Dispatchers.Main) { PairingController(context(), scope, { "synthetic-device" }, {}) { _, _ -> service } }
        try {
            withContext(Dispatchers.Main) { controller.enter() }
            withTimeout(5000) { entered.await() }
            val prior = withContext(Dispatchers.Main) { controller.scanRequestEpoch }
            withContext(Dispatchers.Main) { controller.leave() }
            release.complete(Unit)
            withTimeout(5000) { while (withContext(Dispatchers.Main) { controller.working }) delay(10) }
            withContext(Dispatchers.Main) {
                assertFalse(controller.peersLoaded); assertTrue(controller.peers.isEmpty())
                assertFalse(controller.scanner); assertTrue(controller.scanRequestEpoch != prior)
            }
        } finally { release.complete(Unit); withContext(Dispatchers.Main) { controller.close(); scope.cancel() } }
    }

    @Test fun trustedPeersRemainLoadedWhenOptionalDeviceDetailsFail() = runBlocking {
        val peer = PairedDevice("synthetic-peer", "a".repeat(64), 1uL, null)
        val service = object : FakeSessions() {
            override suspend fun pairedDevices() = listOf(peer)
            override suspend fun localDevice(): SessionDevice = throw SessionFailure(SessionFailureKind.Transport)
        }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val controller = withContext(Dispatchers.Main) { PairingController(context(), scope, { "synthetic-device" }, {}) { _, _ -> service } }
        try {
            withContext(Dispatchers.Main) { controller.enter() }
            withTimeout(5000) { while (withContext(Dispatchers.Main) { controller.working }) delay(10) }
            withContext(Dispatchers.Main) {
                assertTrue(controller.peersLoaded); assertEquals(listOf(peer), controller.peers)
                assertFalse(controller.scanner); assertNotNull(controller.problem)
            }
        } finally { withContext(Dispatchers.Main) { controller.close(); scope.cancel() } }
    }

    @Test fun friendlyNamesStayLocalAndCertificateBoundWithoutChangingIdentityOrRoutes() = runBlocking {
        val directory = namingDirectory()
        val ownedContext = namingContext(directory)
        val first = PairedDevice("synthetic-peer-a", "a".repeat(64), 1uL, null)
        val second = PairedDevice("synthetic-peer-b", "b".repeat(64), 2uL, null)
        val file = File(directory, "peer-labels.properties")
        withContext(Dispatchers.IO) {
            val store = PeerLabelStore(file.toPath())
            assertNull(store.rename(first.deviceId, first.fingerprint, "Studio").issue)
            assertNull(store.rename(second.deviceId, second.fingerprint, "Studio").issue)
        }
        var currentPeers = listOf(second, first)
        val service = object : FakeSessions() { override suspend fun pairedDevices() = currentPeers }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val controller = withContext(Dispatchers.Main) { PairingController(ownedContext, scope, { "synthetic-device" }, {}) { _, _ -> service } }
        try {
            withContext(Dispatchers.Main) { controller.sessionAddress = "192.0.2.10:45450"; controller.enter() }
            waitPairing(controller)
            withContext(Dispatchers.Main) {
                assertEquals(listOf(first, second), controller.activePeers())
                assertEquals("Studio · ${first.fingerprint.take(12)}", controller.peerDisplayName(first))
                assertEquals("Studio · ${second.fingerprint.take(12)}", controller.peerDisplayName(second))
                controller.choosePeer(first.deviceId)
                controller.renamePeer(first.deviceId, first.fingerprint, "Work PC")
            }
            waitNames(controller)
            withContext(Dispatchers.Main) {
                assertEquals("Work PC", controller.peerDisplayName(first))
                assertEquals(first.deviceId, controller.selectedPeer)
                assertEquals("192.0.2.10:45450", controller.sessionAddress)
                currentPeers = listOf(first.copy(fingerprint = "c".repeat(64)), second)
                controller.enter()
            }
            waitPairing(controller)
            withContext(Dispatchers.Main) { assertEquals("Computer 1", controller.peerName(currentPeers.first())) }
            val external = "unknown-existing-naming-bytes".toByteArray()
            withContext(Dispatchers.IO) { file.writeBytes(external) }
            withContext(Dispatchers.Main) { controller.renamePeer(first.deviceId, currentPeers.first().fingerprint, "Other name") }
            waitNames(controller)
            withContext(Dispatchers.Main) {
                assertFalse(controller.namesWritable); assertNotNull(controller.namingProblem)
                assertTrue(controller.peersLoaded); assertEquals(currentPeers, controller.peers)
                assertEquals(first.deviceId, controller.selectedPeer); assertEquals("192.0.2.10:45450", controller.sessionAddress)
            }
            withContext(Dispatchers.IO) { assertArrayEquals(external, file.readBytes()) }
        } finally { withContext(Dispatchers.Main) { controller.close(); scope.cancel() }; disposeNamingDirectory(directory) }
    }

    @Test fun acceptedQrOwnsBytesAndCopiesOnlyChosenRouteAfterSuccessfulPairing() = runBlocking {
        val directory = namingDirectory()
        val peer = PairedDevice("synthetic-peer-a", "a".repeat(64), 1uL, null)
        val other = PairedDevice("synthetic-peer-b", "b".repeat(64), 2uL, null)
        val inspecting = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val expectedEndpoint = "192.0.2.20:443"
        val service = object : FakeSessions() {
            override suspend fun pairedDevices() = listOf(peer, other)
            override suspend fun inspectQr(qr: ByteArray): QrDetails {
                inspecting.complete(Unit); release.await()
                return QrDetails(peer.deviceId, peer.fingerprint, (System.currentTimeMillis() + 100_000).toULong(), listOf(expectedEndpoint))
            }
            override suspend fun joinQr(qr: ByteArray, endpoint: String): String { assertEquals(expectedEndpoint, endpoint); return peer.deviceId }
        }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val controller = withContext(Dispatchers.Main) { PairingController(namingContext(directory), scope, { "synthetic-device" }, {}) { _, _ -> service } }
        val accepted = byteArrayOf(1, 2, 3); val refused = byteArrayOf(4, 5, 6)
        try {
            withContext(Dispatchers.Main) {
                controller.carrier = SessionCarrier.QuicTether; controller.sessionAddress = "192.0.2.10:45450"
                controller.carrier = SessionCarrier.QuicWifi; controller.sessionAddress = "192.0.2.11:45451"
                controller.carrier = SessionCarrier.TcpAdb; controller.sessionAddress = "127.0.0.1:45452"
                controller.carrier = SessionCarrier.QuicWifi
                assertTrue(controller.scan(accepted))
            }
            withTimeout(5000) { inspecting.await() }
            withContext(Dispatchers.Main) { assertFalse(controller.scan(refused)); assertFalse(controller.canUsePairingConnection()) }
            assertArrayEquals(ByteArray(3), accepted); assertArrayEquals(ByteArray(3), refused)
            release.complete(Unit); waitPairing(controller)
            withContext(Dispatchers.Main) { assertFalse(controller.canUsePairingConnection()); controller.pairQr() }
            waitPairing(controller)
            withContext(Dispatchers.Main) {
                assertTrue(controller.canUsePairingConnection())
                assertEquals(listOf("192.0.2.10:45450", "192.0.2.11:45451", "127.0.0.1:45452"), controller.endpoint().map { it.address })
                controller.usePairingConnection()
                assertEquals(listOf("192.0.2.10:45450", expectedEndpoint, "127.0.0.1:45452"), controller.endpoint().map { it.address })
                controller.choosePeer(other.deviceId); assertFalse(controller.canUsePairingConnection())
                controller.choosePeer(peer.deviceId); assertFalse(controller.canUsePairingConnection())
                assertTrue(controller.scan(byteArrayOf(7)))
            }
            waitPairing(controller); withContext(Dispatchers.Main) { controller.pairQr() }; waitPairing(controller)
            withContext(Dispatchers.Main) { assertTrue(controller.canUsePairingConnection()); controller.leave(); assertFalse(controller.canUsePairingConnection()) }
        } finally { release.complete(Unit); withContext(Dispatchers.Main) { controller.close(); scope.cancel() }; disposeNamingDirectory(directory) }
    }

    @Test fun pairingConnectionHintRejectsChangedCertificateAndPendingCodeConfirmation() = runBlocking {
        val directory = namingDirectory()
        var peer = PairedDevice("synthetic-peer", "a".repeat(64), 1uL, null)
        val joinedEndpoints = mutableListOf<String>()
        var confirmations = 0
        val service = object : FakeSessions() {
            override suspend fun pairedDevices() = listOf(peer)
            override suspend fun joinCode(code: String, endpoint: String): PairingConfirmation {
                assertEquals("12345678", code); joinedEndpoints += endpoint
                val expected = peer.fingerprint
                return object : PairingConfirmation {
                    override val fingerprint = expected
                    override suspend fun confirm(displayedFingerprint: String): String { assertEquals(expected, displayedFingerprint); confirmations++; return peer.deviceId }
                    override suspend fun decline() = Unit
                    override suspend fun close() = Unit
                }
            }
            override suspend fun revoke(deviceId: String) { assertEquals(peer.deviceId, deviceId); peer = peer.copy(revokedAtMs = 3uL) }
        }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val controller = withContext(Dispatchers.Main) { PairingController(namingContext(directory), scope, { "synthetic-device" }, {}) { _, _ -> service } }
        try {
            withContext(Dispatchers.Main) { controller.code = "12345678"; controller.pairingEndpoint = "192.0.2.20:443"; controller.pairCode() }
            waitPairing(controller)
            withContext(Dispatchers.Main) {
                assertFalse(controller.canUsePairingConnection()); assertEquals(0, confirmations)
                controller.pairingEndpoint = "192.0.2.99:443"; controller.confirmFingerprint()
            }
            waitPairing(controller)
            withContext(Dispatchers.Main) {
                assertTrue(controller.canUsePairingConnection()); controller.usePairingConnection()
                assertEquals("192.0.2.20:443", controller.sessionAddress); assertEquals(listOf("192.0.2.20:443"), joinedEndpoints)
                peer = peer.copy(fingerprint = "b".repeat(64)); controller.enter()
            }
            waitPairing(controller)
            withContext(Dispatchers.Main) {
                assertFalse(controller.canUsePairingConnection()); controller.usePairingConnection(); assertEquals("192.0.2.20:443", controller.sessionAddress)
                controller.code = "12345678"; controller.pairingEndpoint = "192.0.2.30:443"; controller.pairCode()
            }
            waitPairing(controller); withContext(Dispatchers.Main) { controller.confirmFingerprint() }; waitPairing(controller)
            withContext(Dispatchers.Main) { assertTrue(controller.canUsePairingConnection()); controller.revoke(peer.deviceId) {} }
            waitPairing(controller)
            withContext(Dispatchers.Main) { assertFalse(controller.canUsePairingConnection()); assertNull(controller.selectedPeer); assertEquals(2, confirmations) }
        } finally { withContext(Dispatchers.Main) { controller.close(); scope.cancel() }; disposeNamingDirectory(directory) }
    }

    private suspend fun waitPairing(controller: PairingController) { withTimeout(5000) { while (withContext(Dispatchers.Main) { controller.working }) delay(10) } }
    private suspend fun waitNames(controller: PairingController) { withTimeout(5000) { while (withContext(Dispatchers.Main) { controller.namingWorking }) delay(10) } }
    private fun namingDirectory(): File = File(context().cacheDir.canonicalFile, "vw-peer-labels-test-${UUID.randomUUID()}").also { check(it.mkdir()) }
    private fun namingContext(directory: File) = object : ContextWrapper(context()) { override fun getNoBackupFilesDir(): File = directory }
    private fun disposeNamingDirectory(directory: File) {
        check(directory.parentFile == context().cacheDir.canonicalFile && directory.name.startsWith("vw-peer-labels-test-"))
        check(directory.canonicalFile == directory && directory.deleteRecursively())
    }

    private fun context() = InstrumentationRegistry.getInstrumentation().targetContext
    internal open class FakeSessions : WorkbenchSessions {
        var closes = 0
        override suspend fun localDevice() = SessionDevice("synthetic-device", "0".repeat(64))
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
        override suspend fun close() { closes++ }
    }
}
