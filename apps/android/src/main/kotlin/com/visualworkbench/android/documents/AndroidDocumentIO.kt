package com.visualworkbench.android.documents

import android.content.Context
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import com.visualworkbench.android.editor.ProviderIo
import com.visualworkbench.bindings.core.documentSourceHash
import com.visualworkbench.bindings.core.DocumentJobException
import com.visualworkbench.shared.*
import java.io.ByteArrayOutputStream
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.*

internal class AndroidDocumentIO(context: Context, private val core: WorkbenchCore, private val jobs: DocumentJobs) {
    private val context = context.applicationContext
    private val provider = ProviderIo(this.context.contentResolver)
    private val importGate = Mutex()
    private val closing = AtomicBoolean(false)
    private val activeImport = AtomicReference<Job?>(null)
    private var pendingProject: WorkbenchProject? = null
    private val publisher = DocumentPublisher { bytes -> withContext(Dispatchers.Default) { documentSourceHash(bytes) } }

    suspend fun import(
        uri: Uri, kind: EditableKind, path: String, device: String, title: String,
        password: String? = null, progress: (String) -> Unit,
    ): WorkbenchProject = importGate.withLock {
        if (closing.get()) throw DocumentFailure(DocumentFailureKind.Unavailable)
        if (pendingProject != null) throw DocumentFailure(DocumentFailureKind.Isolation)
        coroutineScope {
            val owner = currentCoroutineContext()[Job]
            activeImport.set(owner)
            try { importOwned(uri, kind, path, device, title, password, progress) }
            catch (error: CancellationException) { throw error }
            catch (error: DocumentFailure) { throw error }
            catch (error: DocumentJobException) { throw nativeDocumentFailure(error) }
            catch (_: Exception) { throw DocumentFailure(DocumentFailureKind.Worker) }
            finally { activeImport.compareAndSet(owner, null) }
        }
    }

    private suspend fun importOwned(
        uri: Uri, kind: EditableKind, path: String, device: String, title: String,
        password: String?, progress: (String) -> Unit,
    ): WorkbenchProject {
        if (uri.scheme != "content") throw DocumentFailure(DocumentFailureKind.Invalid)
        val maximum = if (kind == EditableKind.Svg) 4 * 1024 * 1024 else 64 * 1024 * 1024
        val bytes = withContext(Dispatchers.IO) {
            provider.open(uri, "r").use { input ->
                provider.nonBlocking(input)
                val output = ByteArrayOutputStream(65536); val buffer = ByteArray(65536)
                while (true) {
                    currentCoroutineContext().ensureActive()
                    val count = provider.read(input.fileDescriptor, buffer)
                    if (count == 0) break
                    if (count < 0 || count > maximum - output.size()) throw DocumentFailure(DocumentFailureKind.Limit)
                    output.write(buffer, 0, count)
                }
                if (output.size() == 0) throw DocumentFailure(DocumentFailureKind.Invalid)
                output.toByteArray()
            }
        }
        val inspection = DocumentInteraction(jobs, core).inspect(bytes, kind, password) { done, total -> progress("Reading document pages $done of $total") }
        val documents = core as? WorkbenchDocumentsCore ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
        val now = System.currentTimeMillis()
        if (pendingProject != null) throw DocumentFailure(DocumentFailureKind.Isolation)
        var acquired: WorkbenchProject? = null
        currentCoroutineContext().ensureActive()
        try {
            withContext(NonCancellable) {
                acquired = documents.createDocument(CreateDocument(path, core.newId(now.toULong()), core.newId(now.toULong()), device,
                    title.take(256), bytes, inspection.first.sourceBlake3, kind, inspection.second, now, password))
            }
            currentCoroutineContext().ensureActive()
            val result = requireNotNull(acquired); acquired = null; return result
        } finally { withContext(NonCancellable) {
            acquired?.let { value -> pendingProject = value; value.close(); pendingProject = null }
        } }
    }

    /** uri must be the newly created destination from the caller's current
     * CreateDocument ticket, never an arbitrary pre-existing provider object. */
    suspend fun publish(uri: Uri, copy: DocumentExport, stillCurrent: () -> Boolean) {
        if (uri.scheme != "content") throw DocumentFailure(DocumentFailureKind.Invalid)
        publisher.publish(object : DocumentDestination {
            private var output: ParcelFileDescriptor? = null
            override suspend fun open() {
                check(output == null)
                val acquired = provider.open(uri, "wt")
                output = acquired; provider.nonBlocking(acquired)
            }
            override suspend fun write(bytes: ByteArray, offset: Int, length: Int) {
                val privateChunk = bytes.copyOfRange(offset, offset + length)
                provider.write(requireNotNull(output).fileDescriptor, privateChunk, privateChunk.size)
            }
            override suspend fun finish() { currentCoroutineContext().ensureActive() }
            override suspend fun close() { output?.close(); output = null }
            override suspend fun remove(): Boolean = withTimeout(2000) { provider.perform {
                if (DocumentsContract.isDocumentUri(context, uri)) DocumentsContract.deleteDocument(context.contentResolver, uri)
                else context.contentResolver.delete(uri, null, null) > 0
            } }
        }, copy, stillCurrent)
    }

    suspend fun close() {
        closing.set(true); activeImport.get()?.cancel()
        withContext(NonCancellable) {
            var failure: Exception? = null
            try { publisher.close() } catch (error: Exception) { failure = error }
            try { importGate.withLock { pendingProject?.let { it.close(); pendingProject = null } } }
            catch (error: Exception) { failure = error }
            failure?.let { throw it }
        }
    }
}
