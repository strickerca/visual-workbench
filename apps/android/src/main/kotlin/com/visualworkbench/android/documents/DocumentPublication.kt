package com.visualworkbench.android.documents

import com.visualworkbench.shared.*
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** A capability for ONE newly chosen CreateDocument destination. It must never
 * be constructed from an arbitrary existing provider URI. */
internal interface DocumentDestination {
    suspend fun open()
    suspend fun write(bytes: ByteArray, offset: Int, length: Int)
    suspend fun finish()
    suspend fun close()
    suspend fun remove(): Boolean
}

internal class DocumentPublicationFailure : Exception(
    "The export did not complete. A partial file may remain at the selected location; the original and edits are intact."
)

/** Testable writer/rollback ownership, independent of provider and native code. */
internal class DocumentPublisher(private val hash: suspend (ByteArray) -> String) {
    private data class Pending(val destination: DocumentDestination, val copy: DocumentExport)
    private val gate = Mutex()
    private val closing = AtomicBoolean(false)
    private var pending: Pending? = null

    suspend fun publish(destination: DocumentDestination, copy: DocumentExport, stillCurrent: () -> Boolean) = gate.withLock {
        if (closing.get() || pending != null) {
            withContext(NonCancellable) {
                destination.close()
                if (!removed(destination)) throw DocumentPublicationFailure()
            }
            throw DocumentFailure(if (closing.get()) DocumentFailureKind.Unavailable else DocumentFailureKind.Isolation)
        }
        var complete = false
        try {
            checkCopy(copy)
            current(stillCurrent)
            if (hash(copy.bytes) != copy.blake3) throw DocumentFailure(DocumentFailureKind.Verification)
            current(stillCurrent)
            destination.open()
            var offset = 0
            while (offset < copy.bytes.size) {
                current(stillCurrent)
                val length = minOf(65536, copy.bytes.size - offset)
                destination.write(copy.bytes, offset, length); offset += length
            }
            current(stillCurrent); destination.finish(); destination.close(); current(stillCurrent)
            complete = true
        } finally {
            withContext(NonCancellable) {
                try { destination.close() }
                catch (_: Exception) {
                    pending = Pending(destination, copy)
                    throw DocumentFailure(DocumentFailureKind.Isolation)
                }
                if (!complete && !removed(destination)) throw DocumentPublicationFailure()
            }
        }
    }

    suspend fun close() {
        closing.set(true) // Stops active writes at the next bounded IO boundary.
        withContext(NonCancellable) {
            gate.withLock {
                val retained = pending ?: return@withLock
                retained.destination.close() // Failure retains the exact owner.
                pending = null
                if (!removed(retained.destination)) throw DocumentPublicationFailure()
            }
        }
    }

    private suspend fun current(stillCurrent: () -> Boolean) {
        currentCoroutineContext().ensureActive()
        if (closing.get()) throw DocumentFailure(DocumentFailureKind.Cancelled)
        if (!stillCurrent()) throw DocumentFailure(DocumentFailureKind.Stale)
    }
    private fun checkCopy(copy: DocumentExport) {
        val digest = Regex("[0-9a-f]{64}")
        if (copy.bytes.isEmpty() || copy.bytes.size > 128 * 1024 * 1024) throw DocumentFailure(DocumentFailureKind.Limit)
        if (!digest.matches(copy.blake3) || !digest.matches(copy.sourceAssetId) ||
            copy.redacted != (copy.mode == DocumentCopy.PcRedaction || copy.mode == DocumentCopy.EveryPageRasterRedaction))
            throw DocumentFailure(DocumentFailureKind.Verification)
    }
    private suspend fun removed(destination: DocumentDestination): Boolean = try { destination.remove() } catch (_: Exception) { false }
}
