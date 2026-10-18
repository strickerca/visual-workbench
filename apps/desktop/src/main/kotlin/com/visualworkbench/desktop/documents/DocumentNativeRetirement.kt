package com.visualworkbench.desktop.documents

import com.visualworkbench.shared.*
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Retains one exact native owner until its joined cleanup proof succeeds.
 * An attempted run needs proof regardless of its result/error classification. */
internal class DocumentNativeRetirement(
    private val prove: suspend () -> Unit,
    private val destroy: () -> Unit,
    initiallyRequired: Boolean = false,
) {
    private val state = Any()
    private val retirement = Mutex()
    private var required = initiallyRequired
    private var generation = 0L
    private var cancelled = false
    private var retired = false

    fun cancel(signal: () -> Unit): Unit = synchronized(state) {
        if (!cancelled && !retired) {
            cancelled = true
            try { signal() }
            catch (_: Exception) { required = true; generation++ }
        }
    }

    fun beforeRun(): Unit = synchronized(state) {
        if (retired) throw DocumentFailure(DocumentFailureKind.Isolation)
        if (cancelled) throw DocumentFailure(DocumentFailureKind.Cancelled)
        required = true
        generation++
    }

    suspend fun retire(): Unit = withContext(NonCancellable) {
        retirement.withLock {
            while (true) {
                val (needsProof, proofGeneration) = synchronized(state) {
                    if (retired) return@withLock
                    required to generation
                }
                if (needsProof) {
                    try { prove() }
                    catch (_: Exception) { throw DocumentFailure(DocumentFailureKind.Isolation) }
                }
                val done = synchronized(state) {
                    // A failed cancellation signal during proof needs a fresh
                    // proof. Never suspend while holding the signal lock.
                    if (proofGeneration != generation) false
                    else {
                        try { destroy() }
                        catch (_: Exception) {
                            required = true; generation++
                            throw DocumentFailure(DocumentFailureKind.Isolation)
                        }
                        retired = true
                        required = false
                        true
                    }
                }
                if (done) return@withLock
            }
        }
    }
}
