package com.visualworkbench.shared

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class CaptureChildrenTest {
    private class Child(private val retired: () -> Unit) : OwnedSession {
        var closes = 0
        var fail = false
        override suspend fun close() {
            closes++
            if (fail) { fail = false; error("owned synthetic cleanup") }
            retired()
        }
    }
    @Test fun parentCloseJoinsAllocationAndRejectsItsLatePublication(): Unit = runBlocking {
        val owner = CaptureChildren()
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var child: Child? = null
        val allocation = async { runCatching { owner.acquire { retired ->
            entered.complete(Unit); release.await(); Child(retired).also { child = it }
        } } }
        entered.await()
        val close = launch(start = CoroutineStart.UNDISPATCHED) { owner.close() }
        assertFalse(close.isCompleted)
        release.complete(Unit)
        withTimeout(3000) { close.join() }
        assertTrue(allocation.await().exceptionOrNull() is SessionFailure)
        assertTrue(checkNotNull(child).closes >= 1)
    }
    @Test fun failedCleanupRemainsOwnedAndSecondCloseRetriesIt(): Unit = runBlocking {
        val owner = CaptureChildren()
        val child = owner.acquire { Child(it) }
        child.fail = true
        assertTrue(runCatching { owner.close() }.isFailure)
        assertEquals(1, child.closes)
        owner.close(); assertEquals(2, child.closes)
        owner.close(); assertEquals(2, child.closes)
        assertTrue(runCatching { owner.acquire { Child(it) } }.exceptionOrNull() is SessionFailure)
    }
    @Test fun cancellationJoinsLateAllocationAndRetiresItsChild(): Unit = runBlocking {
        val owner = CaptureChildren()
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var child: Child? = null
        val caller = launch { owner.acquire { retired ->
            entered.complete(Unit); release.await(); Child(retired).also { child = it }
        } }
        entered.await(); caller.cancel(); assertFalse(caller.isCompleted)
        release.complete(Unit); withTimeout(3000) { caller.join() }
        assertNotNull(child); assertEquals(1, checkNotNull(child).closes)
        owner.close(); assertEquals(1, checkNotNull(child).closes)
    }
    @Test fun failedFactorySettlesEmptyAcquisitionBeforeParentClose(): Unit = runBlocking {
        val owner = CaptureChildren()
        assertTrue(runCatching { owner.acquire<Child> { error("synthetic constructor refusal") } }.isFailure)
        withTimeout(3000) { owner.close() }
    }
    @Test fun cancellationBeforeAdmissionCannotStrandPendingParent(): Unit = runBlocking {
        val owner = CaptureChildren()
        val caller = launch(start = CoroutineStart.UNDISPATCHED) {
            currentCoroutineContext().cancel()
            try { owner.acquire { Child(it) } } catch (_: CancellationException) { }
        }
        caller.join()
        withTimeout(3000) { owner.close() }
    }
    @Test fun generatedDestructorFailureIsStickyAndNeverReentered(): Unit = runBlocking {
        val owner = CaptureNativeOwner(); val failure = IllegalStateException("synthetic destructor failure")
        var closes = 0; var destroys = 0
        val first = runCatching { owner.close({ closes++ }, { destroys++; throw failure }) }.exceptionOrNull()
        val second = runCatching { owner.close({ closes++ }, { destroys++ }) }.exceptionOrNull()
        // Coroutine stack recovery may copy the thrown exception at a suspend
        // boundary. Its retained original cause and the ABI call counts matter.
        assertTrue(generateSequence(first) { it.cause }.take(8).any { it === failure })
        assertTrue(generateSequence(second) { it.cause }.take(8).any { it === failure })
        assertEquals(1, closes); assertEquals(1, destroys)
        assertTrue(runCatching { owner.enter() }.exceptionOrNull() is SessionFailure)
    }
}
