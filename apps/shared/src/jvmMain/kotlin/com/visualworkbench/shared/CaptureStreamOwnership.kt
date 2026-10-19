package com.visualworkbench.shared

import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock

private val captureCalls = CoroutineScope(SupervisorJob() + Dispatchers.Default)
private val captureSlots = Semaphore(8)

/** Return/cancel settlement, including cancellation before dispatcher delivery.
 * A caller's cancellation cannot strand a native handle or release a helper
 * packet while native code is still reading its independent copy. */
internal suspend fun <T> joinedCaptureCall(
    cancel: () -> Unit = {}, release: suspend (T) -> Unit = {}, block: suspend () -> T,
): T {
    currentCoroutineContext().ensureActive()
    if (!captureSlots.tryAcquire()) throw SessionFailure(SessionFailureKind.Backpressure)
    try {
        val owned = AtomicReference<Result<T>?>(null)
        val task = captureCalls.async {
            val result = runCatching { block() }
            owned.set(result)
            result.getOrThrow()
        }
        try {
            val value = task.await()
            currentCoroutineContext().ensureActive()
            owned.set(null)
            return value
        } catch (error: CancellationException) {
            try { cancel() } catch (signal: Throwable) { error.addSuppressed(signal) }
            withContext(NonCancellable) {
                try { task.await() } catch (_: Throwable) { }
                val result = owned.getAndSet(null)
                if (result != null && result.isSuccess) {
                    try { release(result.getOrThrow()) } catch (cleanup: Throwable) { error.addSuppressed(cleanup) }
                }
            }
            throw error
        }
    } finally { captureSlots.release() }
}

/** Seal synchronous and suspend calls before close, then retain the generated
 * handle until every entered JNI operation settles. closeNative must wake idle
 * native waits; closure of returned frame owners remains the renderer's duty. */
internal class CaptureNativeOwner {
    private var closing = false
    private var calls = 0
    private var idle: CompletableDeferred<Unit>? = null
    private val closeGate = Mutex()
    private var destroyed = false
    private var destructionFailure: Throwable? = null
    @Synchronized fun enter(): Lease {
        if (closing) throw SessionFailure(SessionFailureKind.Closed)
        calls++
        return Lease(this)
    }
    @Synchronized private fun leave() {
        check(calls > 0)
        calls--
        if (closing && calls == 0) idle?.complete(Unit)
    }
    internal class Lease(private var owner: CaptureNativeOwner?) {
        @Synchronized fun close() { val value = owner ?: return; owner = null; value.leave() }
    }
    fun <T> now(block: () -> T): T { val lease = enter(); try { return block() } finally { lease.close() } }
    suspend fun <T> call(block: suspend () -> T): T {
        currentCoroutineContext().ensureActive()
        val lease = enter()
        try { return block() } finally { lease.close() }
    }
    suspend fun close(closeNative: suspend () -> Unit, destroy: () -> Unit): Unit = withContext(NonCancellable) {
        closeGate.withLock {
            destructionFailure?.let { throw it }
            if (destroyed) return@withLock
            val settled = synchronized(this@CaptureNativeOwner) {
                closing = true
                if (calls == 0) CompletableDeferred(Unit) else idle ?: CompletableDeferred<Unit>().also { idle = it }
            }
            var failure: Throwable? = null
            try { closeNative() } catch (error: Throwable) { failure = error }
            settled.await()
            failure?.let { throw it }
            // A generated destructor may have consumed its pointer before
            // throwing. Retain the failure and never cross that ABI again.
            try { destroy() } catch (error: Throwable) { destructionFailure = error; throw error }
            destroyed = true
        }
    }
}
