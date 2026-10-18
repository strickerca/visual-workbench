package com.visualworkbench.shared

import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

/** Exercises the existing package owner with the verification adapter's nullable
 * compiled-result release contract; it does not load JNI or invent a lifetime. */
class VerificationOwnershipTest {
    @Test fun historyCancellationJoinsTheReadBeforeReturningWithoutDeliveringPrivateNotes(): Unit = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val slots = Semaphore(1)
        val entered = CompletableDeferred<Unit>()
        val finishRead = CompletableDeferred<Unit>()
        val signalled = CompletableDeferred<Unit>()
        val completed = AtomicInteger()
        val delivered = AtomicInteger()
        val destroyed = AtomicInteger()
        val job = launch(start = CoroutineStart.UNDISPATCHED) {
            ownPackageResult<Any, VerificationReviewHistoryPage>(scope, slots, { Any() }, { signalled.complete(Unit) },
                { destroyed.incrementAndGet() }, {}) {
                entered.complete(Unit)
                finishRead.await()
                completed.incrementAndGet()
                VerificationReviewHistoryPage(emptyList(), null)
            }
            delivered.incrementAndGet()
        }
        try {
            withTimeout(5000) { entered.await() }
            job.cancel()
            withTimeout(5000) { signalled.await() }
            assertFalse(job.isCompleted)
            assertEquals(0, completed.get())
            assertEquals(0, destroyed.get())
            assertEquals(0, slots.availablePermits)
            finishRead.complete(Unit)
            withTimeout(5000) { job.join() }
            assertEquals(1, completed.get())
            assertEquals(0, delivered.get())
            assertEquals(1, destroyed.get())
            assertEquals(1, slots.availablePermits)
        } finally {
            finishRead.complete(Unit)
            withContext(NonCancellable) { job.cancelAndJoin() }
            scope.cancel()
        }
    }

    @Test fun cancellationWaitsForTheLateFailurePackageToFinishClosing(): Unit = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val slots = Semaphore(1)
        val entered = CompletableDeferred<Unit>()
        val nativeFinished = CompletableDeferred<Unit>()
        val signalled = CompletableDeferred<Unit>()
        val closing = CompletableDeferred<Unit>()
        val closeFinished = CompletableDeferred<Unit>()
        val closes = AtomicInteger()
        val destroyed = AtomicInteger()
        val compiled = object : WorkbenchCompiledPackage {
            override val info: PackageInfo get() = error("Late result must be closed without inspecting or publishing it")
            override suspend fun close(): Unit {
                closes.incrementAndGet()
                closing.complete(Unit)
                closeFinished.await()
            }
        }
        val job = launch(start = CoroutineStart.UNDISPATCHED) {
            ownPackageResult<Any, WorkbenchCompiledPackage?>(scope, slots, { Any() }, { signalled.complete(Unit) },
                { destroyed.incrementAndGet() }, { it?.close() }) {
                entered.complete(Unit)
                nativeFinished.await()
                compiled
            }
        }
        try {
            withTimeout(5000) { entered.await() }
            job.cancel()
            withTimeout(5000) { signalled.await() }
            assertFalse(job.isCompleted)
            nativeFinished.complete(Unit)
            withTimeout(5000) { closing.await() }
            assertFalse(job.isCompleted)
            assertEquals(0, destroyed.get())
            assertEquals(0, slots.availablePermits)
            closeFinished.complete(Unit)
            withTimeout(5000) { job.join() }
            assertEquals(1, closes.get())
            assertEquals(1, destroyed.get())
            assertEquals(1, slots.availablePermits)
        } finally {
            nativeFinished.complete(Unit)
            closeFinished.complete(Unit)
            withContext(NonCancellable) { job.cancelAndJoin() }
            scope.cancel()
        }
    }
}
