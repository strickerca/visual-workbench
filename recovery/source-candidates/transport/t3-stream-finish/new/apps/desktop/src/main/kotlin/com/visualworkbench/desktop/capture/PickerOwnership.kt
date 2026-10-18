package com.visualworkbench.desktop.capture

import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicReference

/** Retains an uncancellable producer until its actual result settles. A late
 * handle/file owner is released off the UI dispatcher before cancellation
 * returns; cancellation of await is never treated as cancellation of native IO. */
internal suspend fun <T> settledPicker(
    producers: CoroutineScope, release: suspend (T) -> Unit = {}, signal: () -> Unit = {},
    body: suspend () -> T,
): T {
    currentCoroutineContext().ensureActive()
    val delivered = AtomicReference<T?>(null)
    val producer = producers.async { body().also { delivered.set(it) } }
    try {
        val value = producer.await(); currentCoroutineContext().ensureActive()
        delivered.set(null); return value
    } catch (error: CancellationException) {
        try { signal() } catch (failure: Throwable) { error.addSuppressed(failure) }
        withContext(NonCancellable + Dispatchers.Default) {
            try { producer.await() } catch (_: Exception) { }
            finally { delivered.getAndSet(null)?.let { value ->
                try { release(value) } catch (failure: Throwable) { error.addSuppressed(failure) }
            } }
        }
        throw error
    }
}

/** Idempotent synchronous modal lease; it seals peer-follow admission before
 * the window appears and never shares another dialog's release token. */
internal class PickerModal(private val acquire: () -> AutoCloseable) {
    private var lease: AutoCloseable? = null
    fun set(open: Boolean) { if (open) { if (lease == null) lease = acquire() }
        else { val value = lease; lease = null; value?.close() } }
}
