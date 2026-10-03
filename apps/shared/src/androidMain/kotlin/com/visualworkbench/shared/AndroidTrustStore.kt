package com.visualworkbench.shared

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import com.visualworkbench.bindings.core.ProtectedTrustCallback
import com.visualworkbench.bindings.core.SessionException
import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.channels.OverlappingFileLockException
import java.nio.file.Files
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Fixed app no-backup storage, separate from every project/export. The callback
 * is invoked exclusively on native workers. No caller-selected path or key alias
 * is accepted, and unavailable Keystore material is a storage error. */
internal class AndroidTrustStore(context: Context) : ProtectedTrustCallback {
    private val root: File
    private val state: AtomicFile
    private var transient: ByteArray? = null
    init {
        val base = context.applicationContext.noBackupFilesDir.canonicalFile
        root = File(base, "vw-protected-trust")
        check(!Files.isSymbolicLink(root.toPath()))
        check(root.mkdirs() || root.isDirectory)
        check(root.canonicalFile.parentFile == base)
        Os.chmod(root.absolutePath, 0x1c0) // owner rwx, no group/other access
        state = AtomicFile(File(root, "trust.aesgcm"))
    }
    override fun load(): ByteArray? = protected {
        locked {
            val loaded = readState()?.second
            transient?.fill(0)
            transient = loaded
            loaded
        }
    }
    override fun clearLoadedPlaintext() { transient?.fill(0); transient = null }
    override fun compareExchange(expectedRevision: ULong, replacementRevision: ULong, plaintext: ByteArray): Boolean =
        try {
            protected {
                require(plaintext.isNotEmpty() && plaintext.size <= MAX_BYTES)
                require(expectedRevision < ULong.MAX_VALUE && replacementRevision == expectedRevision + 1uL)
                locked {
                    val current = readState()
                    val revision = current?.first ?: 0uL
                    current?.second?.fill(0)
                    if (revision != expectedRevision) return@locked false
                    val aad = ByteBuffer.allocate(16).put(MAGIC).putLong(replacementRevision.toLong()).array()
                    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                    cipher.init(Cipher.ENCRYPT_MODE, key(allowCreate = revision == 0uL))
                    cipher.updateAAD(aad)
                    val encrypted = cipher.doFinal(plaintext)
                    require(cipher.iv.size == 12)
                    val envelope = ByteBuffer.allocate(32 + encrypted.size).put(aad).put(cipher.iv).putInt(encrypted.size).put(encrypted).array()
                    require(envelope.size <= MAX_ENVELOPE)
                    var output: java.io.FileOutputStream? = null
                    try {
                        output = state.startWrite()
                        output.write(envelope)
                        output.fd.sync()
                        state.finishWrite(output)
                        output = null
                        val directory = Os.open(root.absolutePath, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
                        try {
                            check(OsConstants.S_ISDIR(Os.fstat(directory).st_mode))
                            Os.fsync(directory)
                        } finally { Os.close(directory) }
                    } catch (failure: Throwable) {
                        if (output != null) state.failWrite(output)
                        throw failure
                    } finally { encrypted.fill(0); envelope.fill(0) }
                    true
                }
            }
        } finally { plaintext.fill(0) }

    private fun readState(): Pair<ULong, ByteArray>? {
        for (name in listOf("trust.aesgcm", "trust.aesgcm.bak", "trust.aesgcm.new")) {
            require(!Files.isSymbolicLink(File(root, name).toPath()))
        }
        if (!state.baseFile.exists() && !File(root, "trust.aesgcm.bak").exists()) return null
        val bytes = state.openRead().use { input ->
            val length = input.channel.size()
            require(length in 49..MAX_ENVELOPE.toLong())
            val value = ByteArray(length.toInt())
            var offset = 0
            while (offset < value.size) { val count = input.read(value, offset, value.size - offset); check(count > 0); offset += count }
            check(input.read() == -1)
            value
        }
        try {
            val buffer = ByteBuffer.wrap(bytes)
            val magic = ByteArray(8).also(buffer::get)
            require(magic.contentEquals(MAGIC))
            val revision = buffer.long.toULong()
            require(revision > 0uL)
            val iv = ByteArray(12).also(buffer::get)
            val length = buffer.int
            require(length == buffer.remaining() && length in 17..MAX_BYTES + 16)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, key(allowCreate = false), GCMParameterSpec(128, iv))
            cipher.updateAAD(bytes, 0, 16)
            return revision to cipher.doFinal(bytes, 32, length)
        } finally { bytes.fill(0) }
    }
    private fun key(allowCreate: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (store.containsAlias(ALIAS)) return store.getKey(ALIAS, null) as? SecretKey ?: error("key unavailable")
        check(allowCreate)
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setRandomizedEncryptionRequired(true).build())
        }.generateKey()
    }
    private fun <T> locked(action: () -> T): T {
        val file = File(root, "trust.lock")
        require(!Files.isSymbolicLink(file.toPath()))
        return RandomAccessFile(file, "rw").use { owned ->
            val deadline = System.nanoTime() + 5_000_000_000L
            while (true) {
                val lock = try { owned.channel.tryLock() } catch (_: OverlappingFileLockException) { null }
                if (lock != null) return@use lock.use { action() }
                check(System.nanoTime() < deadline)
                Thread.sleep(10)
            }
            @Suppress("UNREACHABLE_CODE") error("unreachable")
        }
    }
    private fun <T> protected(action: () -> T): T = try { action() } catch (_: Exception) {
        clearLoadedPlaintext()
        // Error details may contain local paths/keystore aliases; do not cross
        // the foreign boundary or enter application diagnostics.
        throw SessionException.Storage()
    }
    private companion object {
        const val MAX_BYTES = 2 * 1024 * 1024
        const val MAX_ENVELOPE = MAX_BYTES + 64
        const val ALIAS = "visual-workbench.app-trust.v1"
        val MAGIC: ByteArray = byteArrayOf(86, 87, 84, 82, 85, 83, 84, 49)
    }
}
