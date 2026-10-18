package com.visualworkbench.android.documents

import android.content.*
import android.os.*
import com.visualworkbench.android.editor.ProviderIo
import com.visualworkbench.bindings.core.*
import com.visualworkbench.shared.*
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.*
import kotlinx.coroutines.selects.select

internal fun prepareAndroidDocumentJobs(context: Context): DocumentJobs = JoinedDocumentJobs(AndroidDocumentBackend(context.applicationContext))

private class AndroidDocumentBackend(private val context: Context) : DocumentBackend {
    override val pcRedaction = false
    override fun call(): DocumentCall = AndroidDocumentCall(context)
    override suspend fun close() = Unit // The gate already joined/quarantined every service call.
}

private class AndroidDocumentCall(private val context: Context) : DocumentCall {
    private val connected = CompletableDeferred<IBinder>()
    private val signals = DocumentServiceSignals()
    private val retired = signals.retired
    private val finished = signals.finished
    private val stopped = CompletableDeferred<Unit>()
    private val failed = signals.failed
    private val cancellation = AtomicBoolean(false)
    private val provider = ProviderIo(context.contentResolver)
    @Volatile private var remote: Messenger? = null
    private var binder: IBinder? = null
    private var outputFile: File? = null
    private var pipe: Array<ParcelFileDescriptor>? = null
    private var output: ParcelFileDescriptor? = null
    @Volatile private var writer: Job? = null
    private var bound = false
    private val dispatched: Boolean get() = signals.dispatched
    private var heldSource: ByteArray? = null
    private var released = false
    private val death = IBinder.DeathRecipient { signals.processDied() }
    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, service: IBinder) {
            try {
                service.linkToDeath(death, 0); binder = service
                val messenger = Messenger(service); remote = messenger
                if (cancellation.get()) runCatching { messenger.send(Message.obtain(null, DocumentWire.CANCEL)) }
                connected.complete(service)
            } catch (_: Exception) { failed.complete(DocumentFailureKind.Isolation) }
        }
        override fun onServiceDisconnected(name: ComponentName) { signals.disconnected() }
        override fun onBindingDied(name: ComponentName) { signals.disconnected() }
        override fun onNullBinding(name: ComponentName) { failed.complete(DocumentFailureKind.Isolation) }
        // Only the installed death recipient proves process/Binder retirement.
    }

    override fun cancel() {
        cancellation.set(true); stopped.complete(Unit)
        runCatching { remote?.send(Message.obtain(null, DocumentWire.CANCEL)) }
        writer?.cancel()
    }

    override suspend fun execute(request: DocumentRequest, source: ByteArray): DocumentReply {
        heldSource = source
        if (source.isEmpty()) throw DocumentFailure(DocumentFailureKind.Invalid)
        if (source.size > if (documentSvgOperation(request.operation)) 4 * 1024 * 1024 else DocumentWire.MAX_SOURCE)
            throw DocumentFailure(DocumentFailureKind.Limit)
        val operation = request.operation
        if (operation is DocumentOperation.Copy && operation.kind == DocumentCopy.PcRedaction)
            throw DocumentFailure(DocumentFailureKind.Unsupported)
        try {
            val text = documentRequestJson(request)
            val expected = withContext(Dispatchers.Default) { documentSourceHash(source) }
            if (cancellation.get()) throw DocumentFailure(DocumentFailureKind.Cancelled)
            outputFile = withContext(Dispatchers.IO) { File.createTempFile("vw-doc-job-", ".private", context.cacheDir) }
            pipe = ParcelFileDescriptor.createPipe()
            output = ParcelFileDescriptor.open(requireNotNull(outputFile), ParcelFileDescriptor.MODE_WRITE_ONLY or ParcelFileDescriptor.MODE_TRUNCATE)
            bound = context.bindService(Intent(context, DocumentWorkerService::class.java), connection, Context.BIND_AUTO_CREATE)
            if (!bound) throw DocumentFailure(DocumentFailureKind.Isolation)
            withTimeout(10_000) { select<Unit> {
                connected.onAwait { }
                stopped.onAwait { throw DocumentFailure(DocumentFailureKind.Cancelled) }
                failed.onAwait { throw DocumentFailure(it) }
            } }
            if (cancellation.get()) throw DocumentFailure(DocumentFailureKind.Cancelled)
            val receiver = Messenger(object : Handler(Looper.getMainLooper()) {
                override fun handleMessage(message: Message) { if (message.what == DocumentWire.FINISHED) signals.resultReady() }
            })
            val message = Message.obtain(null, DocumentWire.JOB)
            message.replyTo = receiver
            message.data = Bundle().apply {
                putString("request", text)
                putBoolean("pdfium", !documentSvgOperation(request.operation))
                putParcelable("input", requireNotNull(pipe)[0]); putParcelable("output", requireNotNull(output))
            }
            signals.dispatching() // send can fail after delivering; never infer absence of a worker.
            requireNotNull(remote).send(message)
            requireNotNull(pipe)[0].close(); output?.close(); output = null
            val sourceFd = requireNotNull(pipe)[1]
            provider.nonBlocking(sourceFd)
            writer = CoroutineScope(currentCoroutineContext()).launch(Dispatchers.IO) {
                try {
                    var offset = 0
                    while (offset < source.size) {
                        ensureActive()
                        val count = minOf(65536, source.size - offset)
                        val chunk = source.copyOfRange(offset, offset + count)
                        provider.write(sourceFd.fileDescriptor, chunk, count); offset += count
                    }
                } catch (error: CancellationException) { throw error }
                catch (_: Exception) { failed.complete(DocumentFailureKind.Worker) }
                finally { runCatching { sourceFd.close() } }
            }
            withTimeout(125_000) { select<Unit> {
                finished.onAwait { }
                retired.onAwait { }
                stopped.onAwait { throw DocumentFailure(DocumentFailureKind.Cancelled) }
                failed.onAwait { throw DocumentFailure(it) }
            } }
            withTimeout(5000) { retired.await() } // FINISHED is never a release fence.
            writer?.join()
            if (failed.isCompleted) throw DocumentFailure(failed.await())
            if (cancellation.get()) throw DocumentFailure(DocumentFailureKind.Cancelled)
            val cap = when (request.operation) {
                is DocumentOperation.Tile -> 512 * 512 * 4
                is DocumentOperation.Copy -> DocumentWire.MAX_DATA
                else -> 0
            }
            val value = withContext(Dispatchers.IO) { requireNotNull(outputFile).inputStream().use { DocumentWire.read(it, cap) } }
            val result = DocumentJobResult(value.sourceBlake3, value.metadataJson, value.bytes, value.outputBlake3)
            return withContext(Dispatchers.Default) { documentReply(validateDocumentJobResult(text, expected, result)) }
        } catch (_: TimeoutCancellationException) { throw DocumentFailure(DocumentFailureKind.Timeout) }
        catch (error: DocumentJobException) { throw nativeDocumentFailure(error) }
        catch (error: DocumentFailure) { throw error }
        catch (error: CancellationException) { throw error }
        catch (_: Exception) { throw DocumentFailure(DocumentFailureKind.Worker) }
    }

    override suspend fun retire() {
        if (released) return
        cancel()
        // Unbind asks this one-job service to stop; it does not prove death.
        if (bound) {
            try { context.unbindService(connection); bound = false }
            catch (_: Exception) { throw DocumentFailure(DocumentFailureKind.Isolation) }
        }
        var descriptorFailure = false
        pipe?.forEach { try { it.close() } catch (_: Exception) { descriptorFailure = true } }
        try { output?.close(); output = null } catch (_: Exception) { descriptorFailure = true }
        writer?.cancelAndJoin(); writer = null
        if (dispatched && withTimeoutOrNull(5000) { retired.await(); true } != true)
            throw DocumentFailure(DocumentFailureKind.Isolation) // Retain file, source, binder and death owner.
        if (descriptorFailure) throw DocumentFailure(DocumentFailureKind.Isolation)
        // If no descriptors/job were ever sent, no native worker could borrow
        // this call's source. Otherwise the exact death proof above is required.
        withContext(Dispatchers.IO) {
            outputFile?.let { file ->
                if (file.parentFile.canonicalFile != context.cacheDir.canonicalFile || !file.name.startsWith("vw-doc-job-"))
                    throw DocumentFailure(DocumentFailureKind.Isolation)
                if (file.exists() && !file.delete()) throw DocumentFailure(DocumentFailureKind.Isolation)
            }
        }
        outputFile = null; pipe = null; heldSource = null
        binder?.let { runCatching { it.unlinkToDeath(death, 0) } }; binder = null; remote = null
        released = true
    }
}

internal fun documentSvgOperation(operation: DocumentOperation): Boolean = when (operation) {
    DocumentOperation.SvgInfo -> true
    is DocumentOperation.Tile -> operation.kind == EditableKind.Svg
    is DocumentOperation.Copy -> operation.kind == DocumentCopy.SvgAnnotations
    else -> false
}

internal fun nativeDocumentFailure(error: DocumentJobException): DocumentFailure = DocumentFailure(when (error) {
    is DocumentJobException.Unavailable -> DocumentFailureKind.Unavailable
    is DocumentJobException.Invalid -> DocumentFailureKind.Invalid
    is DocumentJobException.Limit -> DocumentFailureKind.Limit
    is DocumentJobException.Busy -> DocumentFailureKind.Busy
    is DocumentJobException.Cancelled -> DocumentFailureKind.Cancelled
    is DocumentJobException.Timeout -> DocumentFailureKind.Timeout
    is DocumentJobException.Isolation -> DocumentFailureKind.Isolation
    is DocumentJobException.Verification -> DocumentFailureKind.Verification
    is DocumentJobException.Password -> DocumentFailureKind.Password
    is DocumentJobException.Unsupported -> DocumentFailureKind.Unsupported
    is DocumentJobException.Malformed -> DocumentFailureKind.Malformed
    is DocumentJobException.Worker -> DocumentFailureKind.Worker
})
