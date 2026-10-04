package com.visualworkbench.shared

import android.app.Application
import android.system.Os
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import java.io.RandomAccessFile
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Every test creates an isolated synthetic alias and no-backup directory. None
 * obtains AndroidProviderRuntime.credentials or touches the production target. */
@RunWith(AndroidJUnit4::class)
public class ProviderCredentialsInstrumentedTest {
    private val app: Application get() = ApplicationProvider.getApplicationContext()
    private fun fixture(block: (AndroidProviderKeyStore) -> Unit) {
        val store = AndroidProviderKeyStore.fixture(app)
        try { block(store) } finally { store.fixtureAwaitWorkers(); store.disposeFixture() }
    }
    @Test public fun encryptedRoundtripFreshArrayOwnershipAndRemove(): Unit = fixture { store -> runBlocking {
        assertFalse(store.isConfigured())
        val input = "synthetic-only-provider-key".toByteArray()
        val expected = input.copyOf()
        store.save(input)
        assertTrue(input.all { it == 0.toByte() })
        val disk = File(store.fixtureRoot(), "credential.aesgcm").readBytes()
        assertFalse(disk.toList().windowed(expected.size).any { it == expected.toList() })
        val one = requireNotNull(store.loadForNative(5000))
        val two = requireNotNull(store.loadForNative(5000))
        assertNotSame(one, two)
        assertArrayEquals(expected, one)
        store.clearForNative(one)
        assertTrue(one.all { it == 0.toByte() })
        assertArrayEquals(expected, two)
        store.clearForNative(two)
        expected.fill(0)
        store.remove()
        assertFalse(store.isConfigured())
        assertNull(store.loadForNative(5000))
    } }
    @Test public fun invalidBytesAreWipedBeforeStorageMutation(): Unit = fixture { store -> runBlocking {
        for (input in listOf(byteArrayOf(), "key\nheader".toByteArray(), byteArrayOf(-1), ByteArray(4097) { 65 })) {
            try { store.save(input); fail("invalid input accepted") }
            catch (failure: ProviderCredentialException) { assertEquals(ProviderCredentialProblem.Invalid, failure.problem) }
            assertTrue(input.all { it == 0.toByte() })
        }
        assertFalse(store.isConfigured())
    } }
    @Test public fun authenticationFailureIsRedactedAndPreservesCiphertext(): Unit = fixture { store -> runBlocking {
        store.save("synthetic-tamper-fixture".toByteArray())
        val file = File(store.fixtureRoot(), "credential.aesgcm")
        val bytes = file.readBytes().also { it[it.lastIndex] = (it.last().toInt() xor 1).toByte() }
        file.writeBytes(bytes)
        try { store.loadForNative(5000); fail("tampered envelope accepted") }
        catch (failure: ProviderCredentialException) {
            assertEquals(ProviderCredentialProblem.Unavailable, failure.problem)
            assertNull(failure.cause)
        }
        assertArrayEquals(bytes, file.readBytes())
    } }
    @Test public fun missingWrappingKeyDoesNotCreateReplacementForExistingCiphertext(): Unit = fixture { store -> runBlocking {
        store.save("synthetic-wrapping-fixture".toByteArray())
        val file = File(store.fixtureRoot(), "credential.aesgcm")
        val before = file.readBytes()
        store.fixtureDeleteWrappingKey()
        try { store.save("synthetic-new-key".toByteArray()); fail("unreadable old envelope silently replaced") }
        catch (failure: ProviderCredentialException) { assertEquals(ProviderCredentialProblem.Unavailable, failure.problem) }
        assertArrayEquals(before, file.readBytes())
        store.remove()
        store.save("synthetic-explicit-recovery".toByteArray())
        assertTrue(store.isConfigured())
    } }
    @Test public fun malformedLengthAndVersionDoNotTriggerLargeAllocation(): Unit = fixture { store -> runBlocking {
        assertFalse(store.isConfigured())
        val file = File(store.fixtureRoot(), "credential.aesgcm")
        for (bytes in listOf(ByteArray(40), ByteArray(4137), ByteArray(41) { 7 })) {
            file.writeBytes(bytes)
            try { store.loadForNative(5000); fail("malformed envelope accepted") }
            catch (failure: ProviderCredentialException) { assertEquals(ProviderCredentialProblem.Unavailable, failure.problem) }
        }
    } }
    @Test public fun symlinkCiphertextIsRefusedWithoutChangingItsTarget(): Unit = fixture { store -> runBlocking {
        assertFalse(store.isConfigured())
        val sentinel = File(store.fixtureRoot(), "synthetic-sentinel")
        sentinel.writeBytes(byteArrayOf(9, 8, 7))
        val linked = File(store.fixtureRoot(), "credential.aesgcm")
        try {
            Os.symlink(sentinel.absolutePath, linked.absolutePath)
            try { store.remove(); fail("symlink was accepted") }
            catch (failure: ProviderCredentialException) { assertEquals(ProviderCredentialProblem.Unavailable, failure.problem) }
            assertArrayEquals(byteArrayOf(9, 8, 7), sentinel.readBytes())
        } finally { if (java.nio.file.Files.isSymbolicLink(linked.toPath())) assertTrue(linked.delete()); assertTrue(sentinel.delete()) }
    } }
    @Test public fun cancellationWhileLockBlockedCannotPublishLateKey(): Unit = fixture { store -> runBlocking {
        assertFalse(store.isConfigured())
        val input = "synthetic-cancelled-key".toByteArray()
        RandomAccessFile(File(store.fixtureRoot(), "credential.lock"), "rw").use { file ->
            file.channel.lock().use {
                val save = async(start = CoroutineStart.UNDISPATCHED) { store.save(input) }
                save.cancelAndJoin()
                assertTrue(input.all { it == 0.toByte() })
                store.fixtureAwaitWorkers()
                assertFalse(File(store.fixtureRoot(), "credential.aesgcm").exists())
            }
        }
        assertFalse(store.isConfigured())
    } }
    @Test public fun fixturesCannotBootstrapProductionNativeCredentialProvider(): Unit = fixture { store ->
        assertFalse(store.isProductionForNative(app))
        assertEquals(app.noBackupFilesDir.canonicalFile, store.fixtureRoot().canonicalFile.parentFile)
    }
}
