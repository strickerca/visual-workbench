package com.visualworkbench.shared

import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore

/** Pure contract tests: no native library, real files, clipboard or device. */
class MaskContractTest {
    private val binding = SelectionBinding("project", "document", "source", 7u, "visible-a")
    private fun info(hash: String = "visible-a", seq: ULong = 7u): ProjectInfo =
        ProjectInfo("project", "Title", "device", 10u, false, false, seq, hash, listOf("document"))
    private fun edit(operation: SelectionOperation): SelectionEdit = SelectionEdit(binding, "txn", "device", 10u, 1,
        SelectionTarget.New("mask", "layer"), operation)

    @Test fun visibleHashFencesOptimisticChangesAtTheSameHostSequence() {
        assertTrue(binding.matches(info(), "document"))
        assertFalse(binding.matches(info("pending-visible-b"), "document"))
        assertFalse(binding.matches(info(seq = 8u), "document"))
        assertFalse(binding.matches(info(), "another-document"))
        assertFalse(binding.matches(info().copy(projectId = "other-project"), "document"))
    }

    @Test fun aSubmittedPathOwnsItsListBeforeAnySuspension() {
        val caller = mutableListOf(Point(1.0, 2.0), Point(3.0, 4.0), Point(5.0, 6.0))
        val owned = edit(SelectionOperation.Lasso(caller, SelectionCombine.Add)).ownedSelectionRequest()
        caller[0] = Point(99.0, 99.0); caller.clear()
        val operation = owned.operation as SelectionOperation.Lasso
        assertEquals(listOf(Point(1.0, 2.0), Point(3.0, 4.0), Point(5.0, 6.0)), operation.points)
        assertEquals(binding, owned.binding)
    }

    @Test fun invalidPathCountsFailBeforeNativeMarshaling() {
        val tooMany = List(16_385) { Point(0.0, 0.0) }
        for (operation in listOf(SelectionOperation.Lasso(emptyList(), SelectionCombine.Add),
            SelectionOperation.Lasso(tooMany, SelectionCombine.Add),
            SelectionOperation.Paint(emptyList(), 4.0, 255u, SelectionCombine.Subtract))) {
            assertEquals(SelectionFailureKind.Limit,
                assertFailsWith<SelectionFailure> { edit(operation).ownedSelectionRequest() }.kind)
        }
    }

    private class Token(private val signal: () -> Unit = {}) : SelectionCancellation {
        val cancelled = AtomicBoolean(false)
        val released = AtomicBoolean(false)
        override fun cancel() { cancelled.set(true); signal() }
        override fun release() { check(released.compareAndSet(false, true)) }
    }

    @Test fun cancellationWaitsForThePrivateProducerBeforeCallerCleanup() = runBlocking {
        withTimeout(5000) {
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
            val started = CompletableDeferred<Unit>(); val signalled = CompletableDeferred<Unit>()
            val finish = CompletableDeferred<Unit>(); val settled = AtomicBoolean(false)
            val cleanup = AtomicBoolean(false); val token = Token { signalled.complete(Unit) }
            val caller = launch(start = CoroutineStart.UNDISPATCHED) {
                try {
                    settledSelection({ token }, scope, Semaphore(1)) {
                        started.complete(Unit)
                        finish.await()
                        settled.set(true)
                        "private output is now closed"
                    }
                    fail("Cancelled output was delivered")
                } finally {
                    assertTrue(settled.get(), "caller cleanup raced an open producer file")
                    assertTrue(token.released.get())
                    cleanup.set(true)
                }
            }
            try {
                started.await(); caller.cancel(); signalled.await()
                assertFalse(caller.isCompleted); assertFalse(token.released.get()); assertFalse(cleanup.get())
                finish.complete(Unit); caller.join()
                assertTrue(cleanup.get()); assertTrue(token.cancelled.get())
            } finally {
                finish.complete(Unit)
                withContext(NonCancellable) { caller.cancelAndJoin() }
                scope.cancel()
            }
        }
    }

    @Test fun aFailedCancelSignalStillWaitsForTheOwnedProducer() = runBlocking {
        withTimeout(5000) {
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
            val started = CompletableDeferred<Unit>(); val signal = CompletableDeferred<Unit>()
            val finish = CompletableDeferred<Unit>(); val token = Token { signal.complete(Unit); error("signal failed") }
            val caller = launch(start = CoroutineStart.UNDISPATCHED) {
                settledSelection({ token }, scope, Semaphore(1)) { started.complete(Unit); finish.await() }
            }
            try {
                started.await(); caller.cancel(); signal.await()
                assertFalse(caller.isCompleted); assertFalse(token.released.get())
                finish.complete(Unit); caller.join(); assertTrue(token.released.get())
            } finally {
                finish.complete(Unit)
                withContext(NonCancellable) { caller.cancelAndJoin() }
                scope.cancel()
            }
        }
    }

    @Test fun admissionIsBoundedAndDoesNotConstructARejectedToken() = runBlocking {
        withTimeout(5000) {
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default); val slots = Semaphore(1)
            val started = CompletableDeferred<Unit>(); val finish = CompletableDeferred<Unit>()
            val rejectedFactories = AtomicInteger(); val token = Token()
            val caller = async {
                settledSelection({ token }, scope, slots) { started.complete(Unit); finish.await(); 42 }
            }
            try {
                started.await()
                val error = assertFailsWith<SelectionFailure> {
                    settledSelection({ rejectedFactories.incrementAndGet(); Token() }, scope, slots) { 1 }
                }
                assertEquals(SelectionFailureKind.Backpressure, error.kind)
                assertEquals(0, rejectedFactories.get())
                finish.complete(Unit); assertEquals(42, caller.await())
                assertEquals(1, slots.availablePermits); assertTrue(token.released.get()); assertFalse(token.cancelled.get())
            } finally {
                finish.complete(Unit)
                withContext(NonCancellable) { caller.cancelAndJoin() }
                scope.cancel()
            }
        }
    }

    @Test fun constructionAndProducerFailuresReleaseAdmission() = runBlocking {
        withTimeout(5000) {
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default); val slots = Semaphore(1)
            try {
                assertFailsWith<IllegalArgumentException> {
                    settledSelection<Token, Unit>({ throw IllegalArgumentException("synthetic") }, scope, slots) { }
                }
                assertEquals(1, slots.availablePermits)
                val token = Token()
                assertFailsWith<IllegalStateException> {
                    settledSelection({ token }, scope, slots) { error("synthetic producer failure") }
                }
                assertEquals(1, slots.availablePermits); assertTrue(token.released.get())
                assertFalse(token.cancelled.get())
            } finally { scope.cancel() }
        }
    }
}
