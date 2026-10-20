package com.visualworkbench.shared

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Register before allocation. A late native result belongs to the same close
 * fence even if its caller disappeared; failed retirement stays strongly owned. */
internal class CaptureChildren {
    private val children = linkedSetOf<Pending<*>>()
    private var closing = false
    @Synchronized private fun add(child: Pending<*>) {
        if (closing) throw SessionFailure(SessionFailureKind.Closed)
        if (children.size >= 16) throw SessionFailure(SessionFailureKind.Backpressure)
        children.add(child)
    }
    @Synchronized private fun retired(child: Pending<*>) { children.remove(child) }
    suspend fun <T : OwnedSession> acquire(cancel: () -> Unit = {}, create: suspend (() -> Unit) -> T): T {
        val pending = Pending<T>(this)
        add(pending)
        return try {
            val value = joinedCaptureCall(cancel = cancel, release = { _: T -> pending.close() }) {
                try { create { retired(pending) }.also { pending.acquired(it) } }
                catch (error: Throwable) { pending.failed(); throw error }
            }
            synchronized(this) { if (closing) throw SessionFailure(SessionFailureKind.Closed) }
            value
        } catch (error: Throwable) {
            // joinedCaptureCall has joined any admitted task; a failure before
            // admission also needs to settle the registered empty acquisition.
            pending.failed()
            withContext(NonCancellable) {
                try { pending.close() } catch (cleanup: Throwable) { if (cleanup !== error) error.addSuppressed(cleanup) }
            }
            throw error
        }
    }
    suspend fun close(): Unit = withContext(NonCancellable) {
        val active = synchronized(this@CaptureChildren) { closing = true; children.toList() }
        var failure: Throwable? = null
        for (child in active) try { child.close() } catch (error: Throwable) {
            val prior = failure
            if (prior == null) failure = error else if (prior !== error) prior.addSuppressed(error)
        }
        failure?.let { throw it }
    }
    private class Pending<T : OwnedSession>(private val parent: CaptureChildren) {
        private val ready = CompletableDeferred<Unit>()
        private val closeGate = Mutex()
        private var child: T? = null
        fun acquired(value: T) { child = value; ready.complete(Unit) }
        fun failed() { ready.complete(Unit) }
        suspend fun close(): Unit = withContext(NonCancellable) {
            ready.await()
            closeGate.withLock {
                child?.close()
                child = null
                parent.retired(this@Pending)
            }
        }
    }
}
