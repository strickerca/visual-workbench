package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import java.util.concurrent.ConcurrentLinkedQueue
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class GiantImportOwnershipTest {
    private class Payload
    private class Fixture {
        val payload = Payload()
        val registry = GiantImportRetirements()
        val events = mutableListOf<String>()
        var refuse = false
        var refuseDestroy = false
        val owner = GiantImportOwner(payload, { value ->
            assertSame(payload, value); events += "retire"
            if (refuse) throw IllegalStateException("held native retirement")
        }, { value ->
            assertSame(payload, value); events += "destroy"
            if (refuseDestroy) throw IllegalStateException("held foreign destruction")
        }, registry)
    }
    private suspend fun until(test: () -> Boolean) { withTimeout(5000) { while (!test()) yield() } }

    @Test fun retirementFailureRetainsExactOwnerAndSealsCallsUntilActualRetry() = runBlocking {
        val f = Fixture(); f.refuse = true
        repeat(2) { assertTrue(runCatching { f.owner.close() }.isFailure) }
        assertEquals(listOf("retire", "retire"), f.events)
        assertEquals(1, f.registry.retainedCount())
        assertEquals(GiantImageFailureKind.Closed,
            (runCatching { f.owner.call { error("sealed call executed") } }.exceptionOrNull() as GiantImageFailure).kind)
        f.refuse = false; f.registry.drain()
        assertEquals(listOf("retire", "retire", "retire", "destroy"), f.events)
        assertEquals(0, f.registry.retainedCount())
        f.owner.close(); assertEquals(4, f.events.size)
    }
    @Test fun destructionFailureNeverRepeatsSuccessfulNativeRetirement() = runBlocking {
        val f = Fixture(); f.refuseDestroy = true
        assertTrue(runCatching { f.owner.close() }.isFailure)
        assertEquals(1, f.registry.retainedCount())
        f.refuseDestroy = false; f.registry.drain()
        assertEquals(listOf("retire", "destroy", "destroy"), f.events)
        assertEquals(0, f.registry.retainedCount())
    }
    @Test fun concurrentCancelledCloseJoinsActiveCallAndActualRetirementOnce() = runBlocking {
        val registry = GiantImportRetirements()
        val entered = CompletableDeferred<Unit>(); val leave = CompletableDeferred<Unit>()
        val retiring = CompletableDeferred<Unit>(); val finish = CompletableDeferred<Unit>()
        var retires = 0; var destroys = 0
        val owner = GiantImportOwner(Payload(), { retires++; retiring.complete(Unit); finish.await() },
            { destroys++ }, registry)
        val call = launch(start = CoroutineStart.UNDISPATCHED) { owner.call { entered.complete(Unit); leave.await() } }
        entered.await()
        val first = launch(start = CoroutineStart.UNDISPATCHED) { owner.close() }
        val second = launch(start = CoroutineStart.UNDISPATCHED) { owner.close() }
        first.cancel(); assertFalse(first.isCompleted); assertFalse(second.isCompleted); assertEquals(0, retires)
        try {
            leave.complete(Unit); retiring.await()
            assertFalse(first.isCompleted); assertFalse(second.isCompleted); assertEquals(0, destroys)
            finish.complete(Unit); call.join(); first.join(); second.join()
            assertEquals(1, retires); assertEquals(1, destroys)
        } finally { leave.complete(Unit); finish.complete(Unit); call.cancelAndJoin(); first.join(); second.join() }
    }
    @Test fun wrappingFailureClosesTheAlreadyOwnedHandle() = runBlocking {
        prepareCoreLibrary()
        val f = Fixture()
        val failure = runCatching { acquireGiantImport({ f.payload }, { f.owner }) {
            throw IllegalArgumentException("wrapper refused")
        } }.exceptionOrNull()
        assertEquals("wrapper refused", failure?.message)
        assertEquals(listOf("retire", "destroy"), f.events)
        assertEquals(0, f.registry.retainedCount())
    }
    @Test fun wrappingFailurePreservesUndeliveredFailedOwnerForRetry() = runBlocking {
        prepareCoreLibrary()
        val f = Fixture(); f.refuse = true
        val failure = runCatching { acquireGiantImport({ f.payload }, { f.owner }) {
            throw IllegalArgumentException("wrapper refused")
        } }.exceptionOrNull()
        assertEquals("wrapper refused", failure?.message)
        assertTrue(failure?.suppressed?.isNotEmpty() == true)
        assertEquals(listOf("retire"), f.events)
        assertEquals(1, f.registry.retainedCount())
        f.refuse = false; f.registry.drain()
        assertEquals(listOf("retire", "retire", "destroy"), f.events)
    }
    @Test fun cancelledAcquisitionSignalsAndJoinsProducerBeforeRetiringUndeliveredHandle() = runBlocking {
        prepareCoreLibrary()
        val f = Fixture(); val entered = CompletableDeferred<Cancellation>(); val leave = CompletableDeferred<Unit>()
        var wrapped = false
        val call = launch { acquireGiantImport({ token -> entered.complete(token); leave.await(); f.payload },
            { f.owner }) { wrapped = true } }
        val token = entered.await()
        try {
            call.cancel(); until { token.isCancelled() }
            assertFalse(call.isCompleted); assertTrue(f.events.isEmpty())
            leave.complete(Unit); call.join()
            assertFalse(wrapped); assertEquals(listOf("retire", "destroy"), f.events)
        } finally { leave.complete(Unit); call.cancelAndJoin() }
    }
    private class QueuedDispatcher : CoroutineDispatcher() {
        val tasks = ConcurrentLinkedQueue<Runnable>()
        override fun dispatch(context: CoroutineContext, block: Runnable) { tasks.add(block) }
        fun runOne() { checkNotNull(tasks.poll()).run() }
    }
    @Test fun cancelledUndeliveredRetirementFailureStaysHeldWithoutDestruction() = runBlocking {
        prepareCoreLibrary()
        val f = Fixture(); f.refuse = true
        val entered = CompletableDeferred<Cancellation>(); val leave = CompletableDeferred<Unit>()
        var wrapped = false
        val call = launch { runCatching {
            acquireGiantImport({ token -> entered.complete(token); leave.await(); f.payload },
                { f.owner }) { wrapped = true }
        } }
        val token = entered.await()
        try {
            call.cancel(); until { token.isCancelled() }
            assertFalse(call.isCompleted)
            leave.complete(Unit); call.join()
            assertFalse(wrapped); assertEquals(listOf("retire"), f.events)
            assertEquals(1, f.registry.retainedCount())
            f.refuse = false; f.registry.drain()
            assertEquals(listOf("retire", "retire", "destroy"), f.events)
        } finally { leave.complete(Unit); call.cancelAndJoin() }
    }
    @Test fun cancellationAfterNativeCompletionBeforeDispatcherDeliveryStillRetires() = runBlocking {
        prepareCoreLibrary()
        val f = Fixture(); val dispatcher = QueuedDispatcher()
        val entered = CompletableDeferred<Unit>(); val leave = CompletableDeferred<Unit>()
        var wrapped = false
        val call = launch(dispatcher) { acquireGiantImport({ entered.complete(Unit); leave.await(); f.payload },
            { f.owner }) { wrapped = true } }
        dispatcher.runOne(); entered.await(); leave.complete(Unit)
        until { dispatcher.tasks.isNotEmpty() }
        call.cancel()
        withTimeout(5000) { while (!call.isCompleted) {
            if (dispatcher.tasks.isNotEmpty()) dispatcher.runOne() else yield()
        } }
        assertFalse(wrapped); assertEquals(listOf("retire", "destroy"), f.events)
    }
    @Test fun requestAndMetadataStringificationOmitsPathsAndOriginalHashes() {
        assertEquals("GiantOriginalOptions", GiantOriginalOptions("private source", "private work", 1u, 1u, true).toString())
        assertEquals("GiantProjectOptions", GiantProjectOptions("private output", "id", "doc", "layer", "device", "private title", 0, 1u).toString())
        val info = GiantOriginalInfo("private hash", "private digest", 1u, 1u, 1u, 1u, 1u, 1u, "png")
        assertFalse(info.toString().contains("private"))
    }
    @Test fun alreadyCancelledAcquisitionNeverEntersProducerOrOwnerFactory() = runBlocking {
        val f = Fixture(); var created = 0; var owned = 0; var wrapped = 0
        val call = launch(start = CoroutineStart.UNDISPATCHED) {
            currentCoroutineContext().cancel()
            assertTrue(runCatching { acquireGiantImport({ created++; f.payload },
                { owned++; f.owner }) { wrapped++ } }.exceptionOrNull() is CancellationException)
        }
        call.join()
        assertEquals(0, created); assertEquals(0, owned); assertEquals(0, wrapped)
        assertTrue(f.events.isEmpty())
    }
}
