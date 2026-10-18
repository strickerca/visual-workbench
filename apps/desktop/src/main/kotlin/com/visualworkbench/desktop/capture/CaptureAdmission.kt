package com.visualworkbench.desktop.capture

import kotlinx.coroutines.*

/** Owner-UI-dispatcher admission; native work remains owned through settlement.
 * Completion, not the coroutine body, releases the slot after pre-dispatch
 * cancellation. No capture/global-hook/OS action is performed by this owner. */
internal class CaptureAdmission(private val scope: CoroutineScope, private val busy: (Boolean) -> Unit = {}) {
    private var work: Job? = null
    private var sealed = false
    fun start(body: suspend () -> Unit): Boolean {
        if (sealed || !scope.isActive || work?.isCompleted == false) return false
        val next = scope.launch(start = CoroutineStart.LAZY) { body() }
        work = next
        busy(true)
        next.invokeOnCompletion { if (work === next) { work = null; busy(false) } }
        next.start()
        return true
    }
    fun cancel() { work?.cancel() }
    suspend fun shutdown() {
        sealed = true
        withContext(NonCancellable) { work?.cancelAndJoin() }
    }
}
