package com.visualworkbench.desktop.documents

import com.visualworkbench.bindings.core.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Host joins child and restores every grant before returning. Cancellation
 * waits for this retirement; it never releases native pins underneath a job. */
internal class DesktopDocumentJobs(private val runtime: DesktopDocumentRuntime, override val pcRedaction: Boolean,
    private val retained: AutoCloseable) : DocumentJobs {
    private val admission=Mutex(); private var closed=false
    override suspend fun run(request: DocumentRequest, source: ByteArray): DocumentReply = admission.withLock {
        if(closed) throw DocumentFailure(DocumentFailureKind.Unavailable)
        val owner=currentCoroutineContext()[Job]
        val cancellation=DocumentJobCancellation()
        withContext(NonCancellable) {
            val monitor=launch(Dispatchers.Default) { while(isActive) { if(owner?.isActive==false) { cancellation.cancel();return@launch };delay(25) } }
            try {
                val text=documentRequestJson(request)
                val result=runtime.run(text,source,cancellation)
                if(owner?.isActive==false) throw CancellationException("Document job retired after cancellation")
                documentReply(validateDocumentJobResult(text,documentSourceHash(source),result))
            } catch(error: DocumentJobException) { throw nativeDocumentFailure(error) }
            finally { monitor.cancelAndJoin();cancellation.destroy() }
        }
    }
    override suspend fun close(): Unit = withContext(NonCancellable) { admission.withLock { if(!closed) { closed=true;runtime.destroy();retained.close() } } }
}
internal fun nativeDocumentFailure(error: DocumentJobException): DocumentFailure = DocumentFailure(when(error) {
    is DocumentJobException.Unavailable->DocumentFailureKind.Unavailable;is DocumentJobException.Invalid->DocumentFailureKind.Invalid;is DocumentJobException.Limit->DocumentFailureKind.Limit;is DocumentJobException.Busy->DocumentFailureKind.Busy;is DocumentJobException.Cancelled->DocumentFailureKind.Cancelled;is DocumentJobException.Timeout->DocumentFailureKind.Timeout;is DocumentJobException.Isolation->DocumentFailureKind.Isolation;is DocumentJobException.Verification->DocumentFailureKind.Verification;is DocumentJobException.Password->DocumentFailureKind.Password;is DocumentJobException.Unsupported->DocumentFailureKind.Unsupported;is DocumentJobException.Malformed->DocumentFailureKind.Malformed;is DocumentJobException.Worker->DocumentFailureKind.Worker
},error)
