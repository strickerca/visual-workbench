package com.visualworkbench.desktop.documents

import com.visualworkbench.bindings.core.*
import com.visualworkbench.desktop.*
import com.visualworkbench.shared.*
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.*
import java.security.MessageDigest
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Only application-packaged resources are admitted. Missing native artifacts
 * are a capability refusal, never a search of PATH or system library folders. */
internal object DesktopDocumentRuntimeCache {
    private val preparing = Mutex()
    private var pending: DocumentJobs? = null

    suspend fun prepare(local: Path): DocumentJobs = preparing.withLock {
        withContext(NonCancellable) { pending?.let { it.close(); pending = null } }
        var acquired: DocumentJobs? = null
        try {
            val result = withContext(Dispatchers.IO) { prepareAt(local).also { acquired = it } }
            currentCoroutineContext().ensureActive()
            acquired = null
            result
        } finally { withContext(NonCancellable) { acquired?.let { closeRetained(it) } } }
    }

    /** Root shutdown also drains failed/cancelled acquisition ownership. */
    suspend fun closePending() = withContext(NonCancellable) {
        preparing.withLock { pending?.let { it.close(); pending = null } }
    }

    private suspend fun closeRetained(owner: DocumentJobs) {
        pending = owner
        owner.close()
        pending = null
    }

    private suspend fun prepareAt(local: Path): DocumentJobs {
        if (!System.getProperty("os.name", "").startsWith("Windows"))
            throw DocumentFailure(DocumentFailureKind.Unavailable)
        val loader = DesktopDocumentRuntimeCache::class.java.classLoader
        val bytes = loader.getResourceAsStream("vw-document-runtime.sha256")?.use { it.readNBytes(8193) }
            ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
        val manifest = DocumentRuntimeManifest.parse(bytes)
        val guard = WindowsNativeFileGuard()
        val leases = mutableListOf<AutoCloseable>()
        val retained = DocumentRuntimeLeases(leases)
        var opened: DesktopDocumentRuntime? = null
        try {
            currentCoroutineContext().ensureActive()
            val anchor = NativeCacheAnchor.prepare(local, guard); leases += anchor
            val base = anchor.directory.resolve("document-runtime")
            ensureDirectory(base); guard.check(base); leases += guard.pinDirectory(base)
            val scratch = base.resolve("document-worker-private")
            ensureDirectory(scratch); guard.check(scratch); leases += guard.pinDirectory(scratch)
            val directory = base.resolve(manifest.key)
            val created = try { Files.createDirectory(directory); true } catch (_: FileAlreadyExistsException) { false }
            guard.check(directory); leases += guard.pinDirectory(directory)
            if (created) for (entry in manifest.entries) {
                currentCoroutineContext().ensureActive()
                val path = directory.resolve(entry.name)
                loader.getResourceAsStream("document-win32-x86-64/${entry.name}")?.use { input ->
                    FileChannel.open(path, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { output ->
                        val hash = MessageDigest.getInstance("SHA-256"); val buffer = ByteArray(65536); var total = 0L
                        while (true) {
                            currentCoroutineContext().ensureActive()
                            val count = input.read(buffer)
                            if (count < 0) break
                            if (count == 0 || count > entry.size - total) throw DocumentFailure(DocumentFailureKind.Verification)
                            total += count; hash.update(buffer, 0, count)
                            val chunk = ByteBuffer.wrap(buffer, 0, count)
                            while (chunk.hasRemaining()) { currentCoroutineContext().ensureActive(); output.write(chunk) }
                        }
                        if (total != entry.size || documentHex(hash.digest()) != entry.sha256)
                            throw DocumentFailure(DocumentFailureKind.Verification)
                        output.force(true)
                    }
                } ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
            }
            // Never adopt a partial extraction, a redirected name or an extra
            // dependency. Native pinning repeats size/hash/ancestor validation.
            val actual = Files.newDirectoryStream(directory).use { stream -> stream.map { it.fileName.toString() }.toSet() }
            if (actual != manifest.entries.map { it.name }.toSet()) throw DocumentFailure(DocumentFailureKind.Verification)
            for (entry in manifest.entries) {
                val path = directory.resolve(entry.name); guard.check(path)
                if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS) || Files.size(path) != entry.size)
                    throw DocumentFailure(DocumentFailureKind.Verification)
            }
            val options = DocumentRuntimeOptions(directory.toString(), manifest.entries.map {
                DocumentRuntimeArtifact(it.name, it.size.toULong(), documentUnhex(it.sha256))
            })
            // Preserve acquisition across prompt cancellation of generated
            // async bindings, then close it before cancellation can escape.
            withContext(NonCancellable) { opened = openDesktopDocumentRuntime(options) }
            currentCoroutineContext().ensureActive()
            anchor.verify()
            val result = DesktopDocumentJobs(requireNotNull(opened), manifest.pcRedaction, retained, manifest.pdfium)
            opened = null
            return result
        } catch (error: Throwable) {
            val cleanup = object : DocumentJobs {
                private var runtime = opened
                override val pcRedaction = false
                override suspend fun run(request: DocumentRequest, source: ByteArray): DocumentReply = throw DocumentFailure(DocumentFailureKind.Isolation)
                override suspend fun close() { runtime?.destroy(); runtime = null; retained.close() }
            }
            withContext(NonCancellable) { closeRetained(cleanup) }
            when (error) {
                is CancellationException -> throw error
                is DocumentFailure -> throw error
                is DocumentJobException -> throw nativeDocumentFailure(error)
                else -> throw DocumentFailure(DocumentFailureKind.Unavailable)
            }
        }
    }

    private fun ensureDirectory(path: Path) {
        try { Files.createDirectory(path) } catch (_: FileAlreadyExistsException) { }
        if (!Files.isDirectory(path, LinkOption.NOFOLLOW_LINKS)) throw DocumentFailure(DocumentFailureKind.Verification)
    }
}
