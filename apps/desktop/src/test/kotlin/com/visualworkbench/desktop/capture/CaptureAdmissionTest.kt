package com.visualworkbench.desktop.capture

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.util.ArrayDeque
import kotlin.coroutines.CoroutineContext

class CaptureAdmissionTest {
    private class Queued: CoroutineDispatcher() {
        private val tasks = ArrayDeque<Runnable>()
        override fun dispatch(context: CoroutineContext, block: Runnable) { tasks.addLast(block) }
        fun drain() { var count = 0; while (tasks.isNotEmpty()) { check(count++ < 32); tasks.removeFirst().run() } }
    }
    @Test fun cancelBeforeBodyReleasesAdmissionAndNextCaptureRuns() = runBlocking {
        val queue = Queued(); val scope = CoroutineScope(SupervisorJob() + queue)
        val owner = CaptureAdmission(scope); var calls = 0
        try {
            assertTrue(owner.start { calls++ }); owner.cancel(); queue.drain()
            assertEquals(0, calls)
            assertTrue(owner.start { calls++ }); queue.drain(); assertEquals(1, calls)
        } finally { owner.cancel(); queue.drain(); owner.shutdown(); scope.cancel() }
    }
    @Test fun cancelledProducerRetainsSlotAndEveryShutdownJoinsItsCleanup() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val owner = CaptureAdmission(scope); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var cleanup = 0; var first: Job? = null; var second: Job? = null
        try {
            assertTrue(owner.start {
                try { awaitCancellation() }
                finally { withContext(NonCancellable) { entered.complete(Unit); release.await(); cleanup++ } }
            })
            owner.cancel(); withTimeout(2000) { entered.await() }
            assertFalse(owner.start { fail("overlapping capture") })
            first = launch(start = CoroutineStart.UNDISPATCHED) { owner.shutdown() }
            second = launch(start = CoroutineStart.UNDISPATCHED) { owner.shutdown() }
            assertFalse(checkNotNull(first).isCompleted); assertFalse(checkNotNull(second).isCompleted)
            checkNotNull(first).cancel(); assertFalse(checkNotNull(first).isCompleted)
            release.complete(Unit); withTimeout(2000) { checkNotNull(first).join(); checkNotNull(second).join() }
            assertEquals(1, cleanup); assertFalse(owner.start { fail("sealed owner") })
        } finally {
            release.complete(Unit)
            withContext(NonCancellable) { owner.shutdown(); first?.cancelAndJoin(); second?.cancelAndJoin() }
            scope.cancel()
        }
    }
    @Test fun bodyCleanupFailureCannotPermanentlyHoldCaptureAdmission() = runBlocking {
        val errors = ArrayList<Throwable>()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined + CoroutineExceptionHandler { _, error -> errors += error })
        val owner = CaptureAdmission(scope); var calls = 0
        try {
            assertTrue(owner.start { try { calls++ } finally { throw IllegalStateException("fixture cleanup") } })
            assertEquals(1, errors.size)
            assertTrue(owner.start { calls++ }); assertEquals(2, calls)
        } finally { owner.shutdown(); scope.cancel() }
    }
}
