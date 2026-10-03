package com.visualworkbench.android

import android.content.Context
import android.content.ContextWrapper
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.nio.ByteBuffer
import java.nio.file.Files
import java.security.KeyStore
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Real Android Keystore and native pairing, confined to the isolated HIL UID.
 * No camera, discovery, external peer, production trust data or key deletion. */
@RunWith(AndroidJUnit4::class)
class TrustStoreInstrumentedTest {
    private class Fixture {
        private val app = InstrumentationRegistry.getInstrumentation().targetContext
        private val parent = app.noBackupFilesDir.canonicalFile
        val root: File
        val context: Context
        val device = workbenchCore().newDeviceId()
        val encrypted: File get() = File(root, "vw-protected-trust/trust.aesgcm")
        private val sessions = mutableListOf<WorkbenchSessions>()

        init {
            check(app.packageName == "com.visualworkbench.android.hil")
            root = File(parent, "trust-fixture-${UUID.randomUUID()}")
            check(root.mkdir())
            context = object : ContextWrapper(app) {
                override fun getApplicationContext(): Context = this
                override fun getNoBackupFilesDir(): File = root
            }
        }

        suspend fun open(id: String = device): WorkbenchSessions = createAndroidSessions(context, id).also { sessions.add(it) }

        suspend fun close(value: WorkbenchSessions) {
            value.close()
            sessions.remove(value)
        }

        suspend fun clean() = withContext(NonCancellable) {
            var failure: Throwable? = null
            for (session in sessions.toList()) {
                try { close(session) }
                catch (next: Throwable) {
                    val prior = failure
                    if (prior == null) failure = next else prior.addSuppressed(next)
                }
            }
            // Preserve files if a native owner could still be alive. The outer
            // HIL runner will remove its isolated package after reporting failure.
            failure?.let { throw it }
            check(root.canonicalFile.parentFile == parent)
            check(root.name.startsWith("trust-fixture-"))
            root.walkTopDown().forEach { check(!Files.isSymbolicLink(it.toPath())) }
            check(!root.exists() || root.deleteRecursively())
        }
    }

    @Test fun keystoreEnvelopeReopensAndRefusesTamperingAndIdentityReplacement() = runBlocking {
        val fixture = Fixture()
        try {
            withTimeout(30_000) {
                val first = fixture.open()
                val identity = first.localDevice()
                fixture.close(first)
                val encrypted = fixture.encrypted.readBytes()
                assertTrue(encrypted.size in 49..(2 * 1024 * 1024 + 64))
                assertTrue(encrypted.copyOfRange(0, 8).contentEquals("VWTRUST1".toByteArray(Charsets.US_ASCII)))
                val header = ByteBuffer.wrap(encrypted)
                assertTrue(header.getLong(8) > 0)
                assertEquals(encrypted.size - 32, header.getInt(28))
                val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
                val key = keyStore.getKey("visual-workbench.app-trust.v1", null) as SecretKey
                assertNull(key.encoded)
                val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, encrypted.copyOfRange(16, 28)))
                cipher.updateAAD(encrypted, 0, 16)
                val plaintext = cipher.doFinal(encrypted, 32, encrypted.size - 32)
                val deviceBytes = fixture.device.toByteArray(Charsets.US_ASCII)
                try {
                    assertTrue(contains(plaintext, deviceBytes))
                    assertFalse(contains(encrypted, deviceBytes))
                } finally { plaintext.fill(0); deviceBytes.fill(0) }

                val reopened = fixture.open()
                assertTrue(identity == reopened.localDevice())
                fixture.close(reopened)
                val beforeRefusal = fixture.encrypted.readBytes()
                try {
                    val failure = runCatching { fixture.open(workbenchCore().newDeviceId()) }.exceptionOrNull()
                    assertTrue(failure is SessionFailure)
                    assertTrue(beforeRefusal.contentEquals(fixture.encrypted.readBytes()))
                    val damaged = beforeRefusal.copyOf()
                    damaged[damaged.lastIndex] = (damaged.last().toInt() xor 1).toByte()
                    try {
                        fixture.encrypted.writeBytes(damaged)
                        val corruption = runCatching { fixture.open() }.exceptionOrNull()
                        assertTrue(corruption is SessionFailure && corruption.kind == SessionFailureKind.Storage)
                        assertTrue(damaged.contentEquals(fixture.encrypted.readBytes()))
                    } finally { fixture.encrypted.writeBytes(beforeRefusal); damaged.fill(0) }
                    val restored = fixture.open()
                    assertTrue(identity == restored.localDevice())
                    fixture.close(restored)
                } finally { beforeRefusal.fill(0); encrypted.fill(0) }
            }
        } finally { fixture.clean() }
    }

    @Test fun nativeLoopbackPairingAndRevocationPersistThroughKeystoreReopen() = runBlocking {
        val hostFixture = Fixture()
        var clientFixture: Fixture? = null
        var listener: PairingServer? = null
        var offer: PairingOffer? = null
        try {
            val clientData = Fixture().also { clientFixture = it }
            withTimeout(45_000) {
                val host = hostFixture.open()
                val client = clientData.open()
                val hostIdentity = host.localDevice()
                val clientIdentity = client.localDevice()
                val server = host.listenPairing("127.0.0.1:0").also { listener = it }
                val offered = server.offer().also { offer = it }
                coroutineScope {
                    val accepting = async { server.accept() }
                    assertTrue(client.joinQr(offered.qr, offered.endpoint) == hostIdentity.deviceId)
                    val accepted = accepting.await()
                    assertTrue(accepted is PairingResult.Paired && accepted.deviceId == clientIdentity.deviceId)
                }
                offered.clear(); offer = null
                server.close(); listener = null
                hostFixture.close(host); clientData.close(client)

                val reopenedHost = hostFixture.open()
                val reopenedClient = clientData.open()
                assertTrue(hostIdentity == reopenedHost.localDevice())
                assertTrue(clientIdentity == reopenedClient.localDevice())
                val hostPeer = reopenedHost.pairedDevices().single()
                val clientPeer = reopenedClient.pairedDevices().single()
                assertTrue(hostPeer.deviceId == clientIdentity.deviceId && hostPeer.revokedAtMs == null)
                assertTrue(clientPeer.deviceId == hostIdentity.deviceId && clientPeer.revokedAtMs == null)
                reopenedHost.revoke(clientIdentity.deviceId)
                hostFixture.close(reopenedHost)
                val afterRevoke = hostFixture.open()
                val revoked = afterRevoke.pairedDevices().single()
                assertTrue(revoked.deviceId == clientIdentity.deviceId && revoked.revokedAtMs != null)
                assertTrue(hostIdentity == afterRevoke.localDevice())
            }
        } finally {
            withContext(NonCancellable) {
                try { offer?.clear(); listener?.close() }
                finally { try { clientFixture?.clean() } finally { hostFixture.clean() } }
            }
        }
    }

    private fun contains(bytes: ByteArray, needle: ByteArray): Boolean =
        needle.isNotEmpty() && bytes.size >= needle.size && (0..bytes.size - needle.size).any { start ->
            needle.indices.all { bytes[start + it] == needle[it] }
        }
}
