package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicReference

/** The completion handler owns every admitted callback, even when cancellation
 * prevents its dispatched coroutine body from ever starting. Completion means
 * the action has settled all native work, not merely that cancellation was sent.
 * Reply failure closes the host; it never pretends that the peer drained. */
internal class McpOwnerWork(
    private val scope: CoroutineScope,
    private val reply: (String, Map<String, Any?>?) -> Unit,
    private val failedReply: () -> Unit,
) {
    private val lock = Any()
    private val running = mutableMapOf<String, Job>()
    private var sealed = false
    val active: Int get() = synchronized(lock) { running.size }

    fun start(id: String, action: suspend () -> Map<String, Any?>): Boolean = synchronized(lock) {
        if (sealed) throw McpRefused()
        require(!running.containsKey(id))
        if (running.size >= 4) return@synchronized false
        val result = AtomicReference<Map<String, Any?>?>(null)
        val job = scope.launch(start = CoroutineStart.LAZY) {
            try {
                val value = action()
                currentCoroutineContext().ensureActive()
                result.set(value)
            } catch (_: Exception) { /* Only a bounded negative receipt crosses IPC. */ }
        }
        // Publish ownership before attaching the handler: an already-cancelled
        // parent may invoke it immediately. The body never owns these resources.
        running[id] = job
        job.invokeOnCompletion { cause ->
            try { reply(id, if (cause == null) result.get() else null) }
            catch (_: Exception) { failedReply() }
            finally { synchronized(lock) { check(running.remove(id) === job) } }
        }
        job.start()
        true
    }

    fun cancel(id: String) { synchronized(lock) { running[id] }?.cancel() }
    /** Admission is sealed atomically with the snapshot, so shutdown cannot miss
     * an owner request racing its drain. The caller cancels/joins these exact jobs. */
    fun seal(): List<Job> = synchronized(lock) { sealed = true; running.values.toList() }
}
