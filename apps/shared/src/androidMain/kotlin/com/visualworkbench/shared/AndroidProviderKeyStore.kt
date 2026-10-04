package com.visualworkbench.shared

import android.app.Application
import android.os.SystemClock
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.channels.OverlappingFileLockException
import java.nio.file.Files
import java.security.KeyStore
import java.util.UUID
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.SynchronousQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull

public enum class ProviderCredentialProblem { Invalid, Unavailable, Busy }
public class ProviderCredentialException(public val problem: ProviderCredentialProblem) :
    Exception("Provider credential operation failed: ${problem.name}")

/** One fixed production credential, outside project/export/backup storage. No
 * plaintext is retained on this instance. Each native read owns a distinct array. */
public class AndroidProviderKeyStore private constructor(
    private val application: Application,
    private val directoryName: String,
    private val alias: String,
    private val fixture: Boolean,
) {
    private val root: File get() = File(application.noBackupFilesDir.canonicalFile, directoryName)

    /** Ownership of input transfers at entry. It is wiped even on rejection. */
    public suspend fun save(input: ByteArray): Unit {
        val owned = try { validate(input); input.copyOf() } finally { input.fill(0) }
        settings(owned) { stop -> locked(stop) { saveLocked(owned, stop) } }
    }
    public suspend fun remove(): Unit = settings { stop -> locked(stop) {
        checkStop(stop)
        val state = state()
        state.delete()
        check(children().none { it.exists() || Files.isSymbolicLink(it.toPath()) })
        syncDirectory()
        keyStore().deleteEntry(alias)
    } }
    public suspend fun isConfigured(): Boolean = settings { stop -> locked(stop) {
        val bytes = readLocked() ?: return@locked false
        try { checkStop(stop); validate(bytes); true } finally { bytes.fill(0) }
    } }

    /** JNI-only fixed method; caller runs on a bounded native credential worker.
     * The caller must clear this exact returned array after copying, even on error. */
    @Suppress("unused")
    public fun loadForNative(timeoutMillis: Long): ByteArray? = redacted {
        require(timeoutMillis in 1..WAIT_MS)
        locked(AtomicBoolean(false), timeoutMillis) { readLocked() }
    }
    @Suppress("unused")
    public fun clearForNative(bytes: ByteArray): Unit { bytes.fill(0) }
    @Suppress("unused")
    public fun isProductionForNative(app: Application): Boolean = !fixture && app === application &&
        directoryName == DIRECTORY && alias == ALIAS

    private fun saveLocked(bytes: ByteArray, stop: AtomicBoolean) {
        val state = state()
        val hasCiphertext = children().any { it.exists() }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key(allowCreate = !hasCiphertext))
        cipher.updateAAD(MAGIC)
        val encrypted = cipher.doFinal(bytes)
        val iv = cipher.iv
        check(iv.size == 12)
        val envelope = ByteBuffer.allocate(24 + encrypted.size)
            .put(MAGIC).put(iv).putInt(encrypted.size).put(encrypted).array()
        var output: java.io.FileOutputStream? = null
        try {
            checkStop(stop)
            output = state.startWrite()
            Os.fchmod(output.fd, 0x180)
            output.write(envelope)
            output.fd.sync()
            checkStop(stop) // last cancellation point before atomic publication
            state.finishWrite(output)
            output = null
            syncDirectory()
            // AtomicFile may log some publication failures instead of throwing.
            // Verify the saved value before reporting an explicit save success.
            val saved = readLocked() ?: error("Unavailable")
            try { check(saved.contentEquals(bytes)) } finally { saved.fill(0) }
        } catch (failure: Throwable) {
            if (output != null) state.failWrite(output)
            throw failure
        } finally { encrypted.fill(0); envelope.fill(0); iv.fill(0) }
    }
    private fun readLocked(): ByteArray? {
        val state = state()
        if (!state.baseFile.exists() && !File(root, "credential.aesgcm.bak").exists()) return null
        val envelope = state.openRead().use { input ->
            val length = input.channel.size()
            check(length in 41..MAX_ENVELOPE.toLong())
            val bytes = ByteArray(length.toInt())
            try {
                var offset = 0
                while (offset < bytes.size) { val n = input.read(bytes, offset, bytes.size - offset); check(n > 0); offset += n }
                check(input.read() == -1)
                bytes
            } catch (failure: Throwable) { bytes.fill(0); throw failure }
        }
        try {
            val buffer = ByteBuffer.wrap(envelope)
            val magic = ByteArray(8).also(buffer::get)
            check(magic.contentEquals(MAGIC))
            val iv = ByteArray(12).also(buffer::get)
            val size = buffer.int
            check(size == buffer.remaining() && size in 17..MAX_KEY_BYTES + 16)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, key(allowCreate = false), GCMParameterSpec(128, iv))
            cipher.updateAAD(MAGIC)
            val plaintext = cipher.doFinal(envelope, 24, size)
            try { validate(plaintext); return plaintext } catch (failure: Throwable) { plaintext.fill(0); throw failure }
        } finally { envelope.fill(0) }
    }
    private fun state(): AtomicFile {
        checkedRoot()
        children().forEach { child ->
            check(!Files.isSymbolicLink(child.toPath()))
            check(!child.exists() || child.isFile)
            check(child.canonicalFile.parentFile == root.canonicalFile)
        }
        return AtomicFile(File(root, "credential.aesgcm"))
    }
    private fun children(): List<File> = listOf("credential.aesgcm", "credential.aesgcm.new", "credential.aesgcm.bak").map { File(root, it) }
    private fun checkedRoot() {
        val base = application.noBackupFilesDir.canonicalFile
        check(!Files.isSymbolicLink(root.toPath()))
        check(root.mkdirs() || root.isDirectory)
        check(root.canonicalFile.parentFile == base)
        Os.chmod(root.absolutePath, 0x1c0)
    }
    private fun syncDirectory() {
        val fd = Os.open(root.absolutePath, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
        try {
            // O_DIRECTORY is not exposed by Android's public OsConstants API.
            // Validate the opened descriptor before syncing the private directory.
            check(OsConstants.S_ISDIR(Os.fstat(fd).st_mode))
            Os.fsync(fd)
        } finally { Os.close(fd) }
    }
    private fun keyStore(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private fun key(allowCreate: Boolean): SecretKey {
        val store = keyStore()
        if (store.containsAlias(alias)) return store.getKey(alias, null) as? SecretKey ?: error("Unavailable")
        check(allowCreate)
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true).build())
        }.generateKey()
    }
    private fun <T> locked(stop: AtomicBoolean, timeoutMs: Long = WAIT_MS, block: () -> T): T {
        checkedRoot()
        val file = File(root, "credential.lock")
        check(!Files.isSymbolicLink(file.toPath()) && (!file.exists() || file.isFile))
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        RandomAccessFile(file, "rw").use { handle ->
            Os.chmod(file.absolutePath, 0x180)
            while (true) {
                checkStop(stop)
                if (SystemClock.elapsedRealtime() >= deadline) throw ProviderCredentialException(ProviderCredentialProblem.Busy)
                val lock = try { handle.channel.tryLock() } catch (_: OverlappingFileLockException) { null }
                if (lock != null) return lock.use { checkStop(stop); block() }
                Thread.sleep(10)
            }
        }
    }
    private suspend fun <T> settings(owned: ByteArray? = null, operation: (AtomicBoolean) -> T): T {
        // No queued work. If cancellation races with running Keystore I/O, that
        // worker retains its slot and plaintext until it actually returns.
        val claimed = AtomicBoolean(false)
        val stop = AtomicBoolean(false)
        try {
            val completed = withTimeoutOrNull(WAIT_MS) { suspendCancellableCoroutine<SettingResult<T>> { continuation ->
                continuation.invokeOnCancellation { stop.set(true) }
                ACTIVE.incrementAndGet()
                try {
                    WORKERS.execute {
                        try { if (claimed.compareAndSet(false, true)) {
                            try {
                                val result = redacted { checkStop(stop); operation(stop) }
                                continuation.resume(SettingResult(result))
                            } catch (failure: Throwable) { continuation.resumeWithException(failure) }
                            finally { owned?.fill(0) }
                        } } finally { ACTIVE.decrementAndGet() }
                    }
                } catch (_: RejectedExecutionException) {
                    ACTIVE.decrementAndGet()
                    continuation.resumeWithException(ProviderCredentialException(ProviderCredentialProblem.Busy))
                } catch (_: Throwable) {
                    ACTIVE.decrementAndGet()
                    continuation.resumeWithException(ProviderCredentialException(ProviderCredentialProblem.Unavailable))
                }
            } } ?: throw ProviderCredentialException(ProviderCredentialProblem.Busy)
            return completed.value
        } finally {
            stop.set(true)
            if (claimed.compareAndSet(false, true)) owned?.fill(0)
        }
    }
    private fun checkStop(stop: AtomicBoolean) { if (stop.get()) throw CancellationException("Credential operation cancelled") }
    private class SettingResult<T>(val value: T)
    private fun <T> redacted(operation: () -> T): T = try { operation() }
        catch (failure: CancellationException) { throw failure }
        catch (failure: ProviderCredentialException) { throw failure }
        catch (_: Throwable) { throw ProviderCredentialException(ProviderCredentialProblem.Unavailable) }

    internal fun fixtureRoot(): File { check(fixture); return root }
    internal fun fixtureDeleteWrappingKey() { check(fixture); keyStore().deleteEntry(alias) }
    internal fun fixtureAwaitWorkers() {
        check(fixture)
        val deadline = SystemClock.elapsedRealtime() + WAIT_MS
        while (ACTIVE.get() != 0 && SystemClock.elapsedRealtime() < deadline) Thread.sleep(10)
        check(ACTIVE.get() == 0)
    }
    internal fun disposeFixture() {
        check(fixture)
        // Exact fixture namespace only; no recursive cleanup or production alias.
        checkedRoot()
        state().delete()
        keyStore().deleteEntry(alias)
        val lock = File(root, "credential.lock")
        check(!Files.isSymbolicLink(lock.toPath()))
        check(!lock.exists() || lock.delete())
        check(root.delete())
    }
    public companion object {
        public const val MAX_KEY_BYTES: Int = 4096
        private const val MAX_ENVELOPE = MAX_KEY_BYTES + 40
        private const val WAIT_MS = 5_000L
        private const val DIRECTORY = "vw-provider-credentials"
        private const val ALIAS = "visual-workbench.openai.api-key.v1"
        private val MAGIC = byteArrayOf(86, 87, 65, 73, 75, 69, 89, 1)
        private val ACTIVE = AtomicInteger(0)
        private val WORKERS = ThreadPoolExecutor(0, 2, 30, TimeUnit.SECONDS, SynchronousQueue(), { job ->
            Thread(job, "vw-provider-settings").apply { isDaemon = true }
        }, ThreadPoolExecutor.AbortPolicy())
        internal fun production(application: Application): AndroidProviderKeyStore =
            AndroidProviderKeyStore(application, DIRECTORY, ALIAS, false)
        internal fun fixture(application: Application): AndroidProviderKeyStore {
            val nonce = UUID.randomUUID().toString()
            return AndroidProviderKeyStore(application, "vw-provider-fixture-$nonce", "$ALIAS.fixture.$nonce", true)
        }
        private fun validate(bytes: ByteArray) {
            if (bytes.isEmpty() || bytes.size > MAX_KEY_BYTES || bytes.any { it.toInt() !in 33..126 })
                throw ProviderCredentialException(ProviderCredentialProblem.Invalid)
        }
    }
}
