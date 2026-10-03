package com.visualworkbench.android

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
