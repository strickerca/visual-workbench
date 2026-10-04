package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

class WorkflowOwnershipTest {
    @Test fun alreadyCancelledCallerDoesNotCreateTokenOrStartProducer() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val calls = AtomicInteger()
        val slots = Semaphore(1)
        try {
            val job = launch(start = CoroutineStart.LAZY) {
                ownWorkflowResult(scope, slots, { calls.incrementAndGet() }, {}, {}, {}) { calls.incrementAndGet() }
            }
            job.cancel(); job.start(); job.join()
            assertEquals(0, calls.get()); assertEquals(1, slots.availablePermits)
            // Exercise the production entry inside an actually entered, already
            // cancelled coroutine too, rather than relying solely on lazy start.
            val entered = launch {
                currentCoroutineContext().cancel()
                try { ownWorkflowResult(scope, slots, { calls.incrementAndGet() }, {}, {}, {}) { calls.incrementAndGet() } }
                catch (_: CancellationException) { }
            }
            entered.join(); assertEquals(0, calls.get()); assertEquals(1, slots.availablePermits)
        } finally { scope.cancel() }
    }
    @Test fun cancelledLateHandleIsReleasedOnlyAfterProducerDespiteSignalFailure() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val slots = Semaphore(1)
        val started = CompletableDeferred<Unit>(); val finish = CompletableDeferred<Unit>()
        val signalled = CompletableDeferred<Unit>(); val released = AtomicInteger(); val destroyed = AtomicInteger()
        try {
            val call = launch {
                ownWorkflowResult(scope, slots, { Unit }, { signalled.complete(Unit); error("synthetic signal failure") }, { destroyed.incrementAndGet() }, { _:Int -> released.incrementAndGet() }) {
                    started.complete(Unit); finish.await(); 42
                }
            }
            withTimeout(5000) { started.await() }; call.cancel(); withTimeout(5000) { signalled.await() }
            assertEquals(0, released.get()); assertEquals(0, destroyed.get()); assertEquals(0, slots.availablePermits)
            assertFalse(call.isCompleted)
            finish.complete(Unit); withTimeout(5000) { call.join() }
            assertEquals(1, released.get()); assertEquals(1, destroyed.get()); assertEquals(1, slots.availablePermits)
        } finally { finish.complete(Unit); scope.cancel() }
    }
    @Test fun successfulDeliveryTransfersOwnershipExactlyOnce() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val releases = AtomicInteger(); val destroys = AtomicInteger(); val slots = Semaphore(1)
        try {
            val result = ownWorkflowResult(scope, slots, { Unit }, {}, { destroys.incrementAndGet() }, { _:String -> releases.incrementAndGet() }) { "owned" }
            assertEquals("owned", result); assertEquals(0, releases.get()); assertEquals(1, destroys.get()); assertEquals(1, slots.availablePermits)
        } finally { scope.cancel() }
    }
    @Test fun busySlotRefusesBeforeCreatingAnotherToken() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val slots = Semaphore(1); slots.acquire(); val created = AtomicInteger()
        try {
            try { ownWorkflowResult(scope, slots, { created.incrementAndGet() }, {}, {}, {}) { 1 }; fail("expected backpressure") }
            catch (error:WorkflowFailure) { assertEquals(WorkflowFailureKind.Backpressure, error.kind) }
            assertEquals(0, created.get()); assertEquals(0, slots.availablePermits)
        } finally { slots.release(); scope.cancel() }
    }
    @Test fun failedTokenCreationAndProducerFailureReleaseTheirSlots() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val slots = Semaphore(1); val destroyed = AtomicInteger()
        try {
            try { ownWorkflowResult<Unit,Unit>(scope, slots, { error("synthetic create") }, {}, {}, {}) {}; fail("expected create error") }
            catch (_:IllegalStateException) { }
            assertEquals(1, slots.availablePermits)
            try { ownWorkflowResult<Unit,Unit>(scope, slots, { Unit }, {}, { destroyed.incrementAndGet() }, {}) { error("synthetic producer") }; fail("expected producer error") }
            catch (_:IllegalStateException) { }
            assertEquals(1, destroyed.get()); assertEquals(1, slots.availablePermits)
        } finally { scope.cancel() }
    }
    @Test fun throwingUndeliveredCleanupStillSettlesAndDestroysToken() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val slots = Semaphore(1); val started = CompletableDeferred<Unit>(); val finish = CompletableDeferred<Unit>()
        val signalled = CompletableDeferred<Unit>(); val destroyed = AtomicInteger(); val released = AtomicInteger()
        try {
            val call = launch {
                ownWorkflowResult(scope, slots, { Unit }, { signalled.complete(Unit) }, { destroyed.incrementAndGet() }, { _:Int -> released.incrementAndGet(); error("synthetic release") }) { started.complete(Unit); finish.await(); 1 }
            }
            withTimeout(5000) { started.await() }; call.cancel(); withTimeout(5000) { signalled.await() }; finish.complete(Unit)
            withTimeout(5000) { call.join() }; assertEquals(1, released.get()); assertEquals(1, destroyed.get()); assertEquals(1, slots.availablePermits)
        } finally { finish.complete(Unit); scope.cancel() }
    }
}
