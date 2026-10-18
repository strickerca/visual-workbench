package com.visualworkbench.android.editor

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Main-thread confined. A surface stays owned after detachment until its real
 * CanvasFrontBufferedRenderer.release callback has discarded recorded drawing.
 * Clearing a scene or hiding a SurfaceControl is not a release acknowledgment. */
internal class GiantRenderLeases(private val available: () -> Unit) {
    private val entries = LinkedHashSet<Lease>()
    private var retiring = false
    private var pauses = 0

    inner class Lease internal constructor(private val request: () -> Unit) {
        private val completion = CompletableDeferred<Unit>()
        private var requested = false
        private var released = false
        internal fun retire() { if (!requested) { requested = true; request() } }
        internal suspend fun await() { completion.await() }
        fun released(announce: Boolean = true) {
            if (released) return
            released = true
            val wasRetiring = retiring
            // A Main.immediate waiter can resume inside complete(). Retire
            // registry ownership before signaling its observation boundary.
            entries.remove(this)
            completion.complete(Unit)
            if (!wasRetiring && pauses == 0 && announce) available()
        }
    }

    fun acquire(retire: () -> Unit): Lease? {
        if (retiring || pauses != 0 || entries.size >= 2) return null
        return Lease(retire).also(entries::add)
    }

    /** Failures preserve all unacknowledged leases and keep admission sealed.
     * No timeout is treated as permission to recycle a live bitmap. */
    suspend fun retireAll() = withContext(NonCancellable) {
        check(!retiring) { "Renderer retirement already owned" }
        retiring = true
        val captured = entries.toList()
        for (entry in captured) entry.retire()
        for (entry in captured) entry.await()
        check(entries.isEmpty())
        retiring = false
        if (pauses == 0) available()
    }

    /** Seal recreation before retiring the old renderer. Reopen only after
     * native strip encoding settles, including cancellation. Main confined. */
    fun pauseAdmission(): AutoCloseable {
        check(pauses < 4); pauses++
        var closed = false
        return AutoCloseable {
            if (!closed) {
                closed = true; pauses--
                if (pauses == 0 && !retiring) available()
            }
        }
    }
    internal fun retainedCount(): Int = entries.size
}
