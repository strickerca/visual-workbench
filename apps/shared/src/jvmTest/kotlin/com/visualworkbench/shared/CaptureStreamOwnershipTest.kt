package com.visualworkbench.shared

import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class CaptureStreamOwnershipTest {
    @Test fun cancelledLateResourceIsReleasedBeforeCallerSettles(): Unit = runBlocking {
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val disposed = AtomicBoolean(false)
        val caller = launch { joinedCaptureCall(release = { _: Any -> disposed.set(true) }) {
            entered.complete(Unit); release.await(); Any()
        } }
        try {
            withTimeout(5000) { entered.await() }; caller.cancel()
            yield(); assertFalse(caller.isCompleted); assertFalse(disposed.get())
            release.complete(Unit); withTimeout(5000) { caller.join() }; assertTrue(disposed.get())
        } finally { release.complete(Unit); withContext(NonCancellable) { caller.cancelAndJoin() } }
    }
    @Test fun failedCancellationSignalStillJoinsAndReleasesLateHandle(): Unit = runBlocking {
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val disposed = AtomicBoolean(false)
        val caller = launch { joinedCaptureCall(cancel = { throw IllegalStateException("synthetic") }, release = { _: Any -> disposed.set(true) }) {
            entered.complete(Unit); release.await(); Any()
        } }
        try {
            withTimeout(5000) { entered.await() }; caller.cancel(); yield(); assertFalse(caller.isCompleted)
            release.complete(Unit); withTimeout(5000) { caller.join() }; assertTrue(disposed.get())
        } finally { release.complete(Unit); withContext(NonCancellable) { caller.cancelAndJoin() } }
    }
    @Test fun alreadyCancelledCallerCannotStartNativeProducer(): Unit = runBlocking {
        var entered = false
        val job = Job().also { it.cancel() }
        try { withContext(job) { joinedCaptureCall { entered = true } }; fail("cancelled caller") }
        catch (_: CancellationException) { }
        assertFalse(entered)
    }
    @Test fun closeSealsBeforeWakingAndRetainsHandleThroughEnteredCalls(): Unit = runBlocking {
        val owner = CaptureNativeOwner(); val call = owner.enter()
        val closeEntered = CompletableDeferred<Unit>(); var destroyed = false
        val closing = launch { owner.close({ closeEntered.complete(Unit) }, { destroyed = true }) }
        withTimeout(5000) { closeEntered.await() }
        try {
            try { owner.enter(); fail("closed admission") } catch (error: SessionFailure) { assertEquals(SessionFailureKind.Closed, error.kind) }
            assertFalse(destroyed); assertFalse(closing.isCompleted)
            call.close(); withTimeout(5000) { closing.join() }; assertTrue(destroyed)
            owner.close({ fail("duplicate close") }, { fail("duplicate destroy") })
        } finally { call.close(); withContext(NonCancellable) { closing.join() } }
    }
    @Test fun nativeCloseFailureJoinsButRetainsHandleForSealedRetry(): Unit = runBlocking {
        val owner = CaptureNativeOwner(); val call = owner.enter()
        val closeEntered = CompletableDeferred<Unit>(); val complete = CompletableDeferred<Throwable?>(); var destroyed = false
        val closing = launch { complete.complete(runCatching { owner.close({ closeEntered.complete(Unit); throw IllegalStateException("synthetic") }, { destroyed = true }) }.exceptionOrNull()) }
        try {
            withTimeout(5000) { closeEntered.await() }; assertFalse(destroyed); assertFalse(complete.isCompleted)
            call.close(); withTimeout(5000) { closing.join() }; assertFalse(destroyed); assertTrue(complete.await() is IllegalStateException)
            try { owner.enter(); fail("failed close reopened admission") } catch (error: SessionFailure) { assertEquals(SessionFailureKind.Closed,error.kind) }
            owner.close({}, { destroyed=true }); assertTrue(destroyed)
            owner.close({fail("duplicate native close")},{fail("duplicate destruction")})
        } finally { call.close(); withContext(NonCancellable) { closing.join() } }
    }
}