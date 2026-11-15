package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProtectedTrustCallback
import com.visualworkbench.bindings.core.openCallbackSessionService
import java.nio.file.Files
import java.util.Base64
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

/** The automatic-connection calls through the real native core, on loopback
 * only. Trust lives in test memory; no address is typed after pairing. */
class NativeAutomaticSessionsTest {
    private class MemoryTrust : ProtectedTrustCallback {
        private var revision = 0uL
        private var state: ByteArray? = null
        @Synchronized override fun load(): ByteArray? = state?.copyOf()
        override fun clearLoadedPlaintext() = Unit
        @Synchronized override fun compareExchange(expectedRevision: ULong, replacementRevision: ULong, plaintext: ByteArray): Boolean {
            if (revision != expectedRevision) return false
            revision = replacementRevision; state = plaintext.copyOf()
            return true
        }
    }
    private fun addresses(endpoints: List<SessionEndpoint>) = endpoints.map { it.address }
    private suspend fun connected(link: ProjectLink): SessionStatus = withTimeout(30_000) {
        var status = link.status()
        while (status.status != SyncStatus.Synced) { delay(10); status = link.status() }
        status
    }

    @Test fun pairedDevicesConnectWithoutATypedAddressThroughThePublicApi() = runBlocking {
        val root = Files.createTempDirectory("vw-automatic-sessions-")
        val core = workbenchCore()
        val now = 1_700_000_000_000L
        val image = Base64.getDecoder().decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQ1QoBAADrAJRe4xMbAAAAAElFTkSuQmCC")
        val computerId = core.newDeviceId(); val phoneId = core.newDeviceId()
        prepareCoreLibrary()
        val computer = sessionFactory { openCallbackSessionService(computerId, MemoryTrust()) }
        val phone = sessionFactory { openCallbackSessionService(phoneId, MemoryTrust()) }
        var project: WorkbenchProject? = null; var copy: WorkbenchProject? = null
        try {
            // Pair once.
            val pairing = computer.listenPairing("127.0.0.1:0")
            val offer = pairing.offer(false)
            val accepted = async { pairing.accept() }
            assertEquals(computerId, phone.joinQr(offer.qr, offer.endpoint))
            assertEquals(PairingResult.Paired(phoneId), accepted.await())
            pairing.close()

            project = core.create(CreateProject(root.resolve("computer").toString(), core.newId(now.toULong()),
                core.newId((now + 1).toULong()), core.newId((now + 2).toULong()), computerId, "Synthetic automatic", image, now))
            // The computer listens by itself and keeps accepting while connected.
            val server = computer.hostAutomatic(project, phoneId, listOf(SessionEndpoint(SessionCarrier.QuicWifi, "127.0.0.1:0")))
            val listening = server.endpoints
            assertEquals(1, listening.size)
            assertEquals(addresses(listening), addresses(server.listenOn(listening)))
            assertTrue(phone.knownRoutes(computerId).isEmpty())

            // First contact uses one hint; afterwards the route is remembered.
            copy = phone.receiveProjectAutomatic(root.resolve("phone").toString(), computerId, listening)
            assertEquals(addresses(listening), addresses(phone.knownRoutes(computerId)))
            val link = phone.connectAutomatic(copy, computerId)
            assertEquals(SessionCarrier.QuicWifi, connected(link).carrier)
            assertEquals(addresses(listening), addresses(link.endpoints))
            link.offerRoutes(emptyList())
            link.close()

            // Saved hints come back only for a paired device, and without one
            // the link waits offline instead of claiming a connection.
            phone.forgetRoutes(computerId)
            assertTrue(phone.knownRoutes(computerId).isEmpty())
            val waiting = phone.connectAutomatic(copy, computerId)
            val idle = withTimeout(30_000) {
                var status = waiting.status()
                while (status.failure != "no_known_route") { delay(10); status = waiting.status() }
                status
            }
            assertNull(idle.carrier)
            assertTrue(idle.status == SyncStatus.Reconnecting || idle.status == SyncStatus.Offline)
            assertNotNull(ErrorCatalog.link(idle.failure))
            assertTrue(readCoreEvents().events.any { it.kind == CoreEventKind.Failure && it.area == "session" && it.name == "no known route" })
            waiting.offerRoutes(listening)
            assertEquals(SessionCarrier.QuicWifi, connected(waiting).carrier)
            waiting.close()
            val stranger = core.newDeviceId()
            assertEquals(SessionFailureKind.Authentication,
                runCatching { phone.restoreRoutes(stranger, listening) }.exceptionOrNull().let { (it as SessionFailure).kind })
            phone.restoreRoutes(computerId, listening)
            assertEquals(addresses(listening), addresses(phone.knownRoutes(computerId)))
            // The listening side has no hints to take; it refuses rather than guesses.
            assertEquals(SessionFailureKind.Invalid,
                runCatching { server.offerRoutes(listening) }.exceptionOrNull().let { (it as SessionFailure).kind })
            server.close()
        } finally {
            withContext(NonCancellable) {
                phone.close(); computer.close()
                copy?.close(); project?.close()
                check(root.toFile().deleteRecursively())
            }
        }
    }
}
