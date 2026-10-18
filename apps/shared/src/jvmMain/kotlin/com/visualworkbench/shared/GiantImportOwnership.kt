package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

internal interface GiantImportRetirement { suspend fun close() }

/** Unknown retirement keeps the exact owner, including an undelivered handle.
 * Drain calls run outside this registry's mutex; close may remove itself. */
internal class GiantImportRetirements {
    private val gate = Mutex()
    private val held = linkedSetOf<GiantImportRetirement>()
    suspend fun retain(owner: GiantImportRetirement) { gate.withLock { held.add(owner) } }
    suspend fun forget(owner: GiantImportRetirement) { gate.withLock { held.remove(owner) } }
    suspend fun drain() {
        val snapshot = gate.withLock { held.toList() }
        snapshot.forEach { it.close() }
    }
    suspend fun retainedCount(): Int = gate.withLock { held.size }
}

internal class GiantImportOwner<T : Any>(val value: T,
    private val retire: suspend (T) -> Unit, private val destroy: (T) -> Unit,
    private val failures: GiantImportRetirements) : GiantImportRetirement {
    private val gate = Mutex()
    private var sealed = false
    private var retired = false
    private var destroyed = false
    suspend fun <R> call(block: suspend (T) -> R): R {
        currentCoroutineContext().ensureActive()
        return gate.withLock {
            currentCoroutineContext().ensureActive()
            if (sealed) throw GiantImageFailure(GiantImageFailureKind.Closed)
            block(value)
        }
    }
    override suspend fun close(): Unit = withContext(NonCancellable) {
        try {
            gate.withLock {
                sealed = true
                if (!retired) { retire(value); retired = true }
                if (!destroyed) { destroy(value); destroyed = true }
            }
            failures.forget(this@GiantImportOwner)
        } catch (error: Throwable) {
            failures.retain(this@GiantImportOwner)
            throw error
        }
    }
}

/** The independent native task is joined on cancellation. Ownership is installed
 * before wrapping; wrapping/cancellation failure closes or retains that owner. */
internal suspend fun <N : Any, R> acquireGiantImport(
    create: suspend (Cancellation) -> N,
    own: (N) -> GiantImportOwner<N>,
    wrap: (GiantImportOwner<N>) -> R,
): R {
    currentCoroutineContext().ensureActive()
    val native = settledStream(release = { own(it).close() }, block = create)
    val owner = own(native)
    try {
        currentCoroutineContext().ensureActive()
        return wrap(owner)
    } catch (error: Throwable) {
        try { owner.close() } catch (cleanup: Throwable) {
            if (cleanup !== error) error.addSuppressed(cleanup)
        }
        throw error
    }
}
