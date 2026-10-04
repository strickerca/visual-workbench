package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import java.util.concurrent.atomic.AtomicReference

/** One owned producer per acquired slot; token/undelivered handle destruction
 * happens only after that producer settles. Kept free of JNI for gated tests. */
internal suspend fun <C, T> ownWorkflowResult(
    scope: CoroutineScope,
    slots: Semaphore,
    create: () -> C,
    signal: (C) -> Unit,
    destroy: (C) -> Unit,
    release: (T) -> Unit,
    block: suspend (C) -> T,
): T {
    currentCoroutineContext().ensureActive()
    if (!slots.tryAcquire()) throw WorkflowFailure(WorkflowFailureKind.Backpressure)
    try {
        currentCoroutineContext().ensureActive()
        val token = create()
        try {
            val owned = AtomicReference<T?>(null)
            val producer = scope.async { block(token).also { owned.set(it) } }
            try {
                val value = producer.await()
                currentCoroutineContext().ensureActive()
                owned.set(null)
                return value
            } catch (error: CancellationException) {
                try { signal(token) } catch (signalError: Exception) { error.addSuppressed(signalError) }
                withContext(NonCancellable) {
                    try { producer.await() } catch (_: Exception) { /* Producer has settled. */ }
                    try { owned.getAndSet(null)?.let(release) } catch (cleanup: Exception) { error.addSuppressed(cleanup) }
                }
                throw error
            }
        } finally { destroy(token) }
    } finally { slots.release() }
}
