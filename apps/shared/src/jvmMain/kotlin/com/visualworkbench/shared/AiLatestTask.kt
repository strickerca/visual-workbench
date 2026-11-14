package com.visualworkbench.android.editor

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel

/** One active producer plus one overwritten intent, even if a native producer
 * takes time to acknowledge cancellation. Owner UI dispatcher only. */
internal class AiLatestTask(private val scope: CoroutineScope, private val failed: (Exception) -> Unit = {}) {
    private val wake = Channel<Unit>(Channel.CONFLATED)
    private var pending: (suspend () -> Unit)? = null
    private var active: Deferred<Unit>? = null
    private var closed = false
    private val runner = scope.launch(start = CoroutineStart.LAZY) {
        for (ignored in wake) {
            val work = pending ?: continue
            pending = null
            try {
                supervisorScope {
                    val task = async(start = CoroutineStart.LAZY) { work() }
                    active = task
                    try { task.start(); task.await() }
                    finally { if (active === task) active = null }
                }
            } catch (cancel: CancellationException) {
                currentCoroutineContext().ensureActive()
            } catch (error: Exception) {
                // An unavailable read must not cancel the editor lifetime or
                // strand the next overwritten intent.
                failed(error)
            }
        }
    }
    fun offer(work: suspend () -> Unit) {
        if (closed || !scope.isActive) return
        pending = work
        active?.cancel()
        wake.trySend(Unit)
        runner.start()
    }
    fun cancelPending() { pending = null; active?.cancel() }
    suspend fun pause() {
        cancelPending()
        withContext(NonCancellable) { active?.join() }
    }
    suspend fun close() {
        closed = true; pending = null; wake.close(); active?.cancel()
        withContext(NonCancellable) { runner.cancelAndJoin() }
    }
}
