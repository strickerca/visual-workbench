package com.visualworkbench.desktop.capture

import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

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
        else { lease?.close(); lease = null } }
}


/** UI-dispatcher admission and generation fence. Retirement seals admission
 * before suspending and remains uncancellable through the actual native join. */
internal class PickerOperationOwner(
    private val scope: CoroutineScope,
    private val busy: (Boolean) -> Unit,
) {
    private var work: Job? = null
    private val retirement = Mutex()
    private var generation = 0L
    private var closing = false
    private var sealed = false
    val accepting: Boolean get() = !closing && !sealed && scope.isActive
    val stopping: Boolean get() = sealed
    fun current(ticket: Long): Boolean = accepting && ticket == generation
    fun invalidate() { generation++; work?.cancel() }
    fun start(body: suspend (Long) -> Unit): Boolean {
        if (!accepting || work?.isCompleted == false) return false
        val ticket = ++generation
        val next = scope.launch(start = CoroutineStart.LAZY) { body(ticket) }
        work = next
        busy(true)
        next.invokeOnCompletion { if (work === next) { work = null; busy(false) } }
        next.start()
        return true
    }
    suspend fun retire(permanent: Boolean = false, cleanup: suspend () -> Unit) {
        // Two callers may be suspended in retirement. Count them so the first
        // completion cannot reopen admission while the second still owns IO.
        retiring++
        closing = true
        if (permanent) sealed = true
        invalidate()
        try { withContext(NonCancellable) { retirement.withLock { work?.cancelAndJoin(); cleanup() } } }
        finally { retiring--; closing = retiring != 0 }
    }
    private var retiring = 0
}


/** A throwing native destructor has an unknown outcome. Keep its owner and
 * failure; repeating a no-op destructor cannot prove the first call retired. */
internal class PickerReleaseAction(private val release: () -> Unit) {
    private var completed = false
    private var failure: Throwable? = null
    fun close() {
        failure?.let { throw it }
        if (completed) return
        try { release(); completed = true }
        catch (error: Throwable) { failure = error; throw error }
    }
}
