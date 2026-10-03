package com.visualworkbench.android.editor

import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.yield

/** Main-dispatcher confined admission, independent of Compose snapshot delivery.
 * The UI's busy/pending setters publish here synchronously. No polling, observer
 * registration, frame clock, background worker or native handle is owned here. */
internal class EditorReadiness {
    private data class State(val busy: Boolean = true, val pending: Int = 0, val closed: Boolean = false) {
        val ready: Boolean get() = !busy && pending == 0
    }
    private val state = MutableStateFlow(State())

    fun busy(value: Boolean) { state.value = state.value.copy(busy = value) }
    fun pending(value: Int) {
        require(value >= 0)
        state.value = state.value.copy(pending = value)
    }
    fun close() { state.value = state.value.copy(closed = true) }

    suspend fun awaitReady(): Boolean {
        while (true) {
            val offered = state.first { it.closed || it.ready }
            if (offered.closed) return false
            // Let the publisher finish its current finally block before a new
            // import takes transferJob/transferLabel ownership. StateFlow can
            // resume a Main.immediate waiter inside the publishing call stack.
            yield()
            currentCoroutineContext().ensureActive()
            val current = state.value
            if (current.closed) return false
            if (current.ready) return true
            // Another operation won that turn; wait for a fresh idle offer.
        }
    }
}
