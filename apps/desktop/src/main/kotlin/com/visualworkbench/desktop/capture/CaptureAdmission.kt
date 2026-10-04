package com.visualworkbench.desktop.capture

import kotlinx.coroutines.*

/** Owner-UI-dispatcher admission; native work remains owned through settlement.
 * Completion, not the coroutine body, releases the slot after pre-dispatch
 * cancellation. No capture/global-hook/OS action is performed by this owner. */
internal class CaptureAdmission(private val scope: CoroutineScope) {
    private var work: Job? = null
    private var sealed = false
    fun start(body: suspend () -> Unit): Boolean {
        if (sealed || !scope.isActive || work?.isCompleted == false) return false
        val next = scope.launch(start = CoroutineStart.LAZY) { body() }
        work = next
        next.invokeOnCompletion { if (work === next) work = null }
        next.start()
        return true
    }
    fun cancel() { work?.cancel() }
    suspend fun shutdown() {
        sealed = true
        withContext(NonCancellable) { work?.cancelAndJoin() }
    }
}
