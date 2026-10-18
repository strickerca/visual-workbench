package com.visualworkbench.shared

import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** App UI-dispatcher owner, injected into document interactions. No process-wide
 * transport singleton or fallback engine is installed by importing these DTOs. */
public class DocumentJobsOwner : DocumentJobs {
    private var jobs: DocumentJobs? = null
    private var closing: Boolean = false
    private val closeMutex: Mutex = Mutex()

    public val held: Boolean get() = jobs != null
    public override val pcRedaction: Boolean get() = !closing && jobs?.pcRedaction == true

    /** A different transport cannot replace one which still owns work or pins. */
    public fun install(value: DocumentJobs): Unit {
        check(!closing && (jobs == null || jobs === value))
        jobs = value
    }

    public override suspend fun run(request: DocumentRequest, source: ByteArray): DocumentReply {
        if (closing) throw DocumentFailure(DocumentFailureKind.Unavailable)
        val current = jobs ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
        return current.run(request, source)
    }

    public override suspend fun close(): Unit {
        closing = true // Refuse new jobs before the first suspension.
        withContext(NonCancellable) {
            closeMutex.withLock {
                val current = jobs ?: return@withLock
                current.close() // Failure retains the actual owner for retry.
                jobs = null
            }
        }
    }
}
