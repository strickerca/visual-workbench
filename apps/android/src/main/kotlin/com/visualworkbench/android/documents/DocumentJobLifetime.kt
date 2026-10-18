package com.visualworkbench.android.documents

import com.visualworkbench.shared.*
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** A call owns its cancellation token, source and any unretired private output.
 * retire must either join/release them or retain the SAME owner and throw
 * Isolation. Returning from execute alone is not a retirement assertion. */
internal interface DocumentCall {
    fun cancel()
    suspend fun execute(request: DocumentRequest, source: ByteArray): DocumentReply
    suspend fun retire()
}

internal interface DocumentBackend {
    val pcRedaction: Boolean
    fun call(): DocumentCall
    suspend fun close()
}

/** close signals the active owner before waiting for run admission. A failed
 * retirement quarantines that owner; replacement calls cannot bypass it. */
internal class JoinedDocumentJobs(private val backend: DocumentBackend) : DocumentJobs {
    private val gate = Mutex()
    private val closing = AtomicBoolean(false)
    private val active = AtomicReference<DocumentCall?>(null)
    private var closed = false

    override val pcRedaction: Boolean get() = !closing.get() && backend.pcRedaction

    override suspend fun run(request: DocumentRequest, source: ByteArray): DocumentReply = gate.withLock {
        if (closing.get()) throw DocumentFailure(DocumentFailureKind.Unavailable)
        if (active.get() != null) throw DocumentFailure(DocumentFailureKind.Isolation)
        val caller = currentCoroutineContext()[Job]
        withContext(NonCancellable) {
            val call = backend.call()
            active.set(call)
            val monitor = launch(Dispatchers.Default) {
                while (isActive) {
                    if (caller?.isActive == false || closing.get()) { call.cancel(); return@launch }
                    delay(10)
                }
            }
            try {
                if (caller?.isActive == false || closing.get()) call.cancel()
                val result = call.execute(request, source)
                if (caller?.isActive == false) throw CancellationException("Document call cancelled after retirement")
                if (closing.get()) throw DocumentFailure(DocumentFailureKind.Cancelled)
                result
            } finally {
                monitor.cancelAndJoin()
                call.retire() // Failure retains call, source, pins and scratch.
                active.compareAndSet(call, null)
            }
        }
    }

    override suspend fun close() {
        closing.set(true)
        active.get()?.cancel()
        withContext(NonCancellable) {
            gate.withLock {
                if (closed) return@withLock
                active.get()?.let { call -> call.cancel(); call.retire(); active.compareAndSet(call, null) }
                backend.close()
                closed = true
            }
        }
    }
}
