package com.visualworkbench.android.remote

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class RemoteDecoderStartupTest {
    private fun deadline(): Long = System.nanoTime() + START_DEADLINE_MILLIS * 1_000_000L
    private fun ready(): DecoderStatus.Ready = DecoderStatus.Ready(
        DecoderAdmission("owned-fixture", true, false, true, true, true, emptyList()))

    @Test fun firstStaticIdrWaitsForReadyWithoutASecondFrame() = runBlocking {
        val state = MutableStateFlow<DecoderStatus>(DecoderStatus.Starting)
        val exactTicket = 17uL
        var retainedTicket: ULong? = exactTicket
        val queued = mutableListOf<ULong>()
        val waiting = async(start = CoroutineStart.UNDISPATCHED) {
            val result = awaitRemoteDecoderStartup(state, deadline()) { true }
            if (result == RemoteDecoderStartup.Ready) queued += requireNotNull(retainedTicket)
            result
        }
        assertFalse(waiting.isCompleted)
        assertEquals(exactTicket, retainedTicket)
        assertTrue(queued.isEmpty())
        // Only worker readiness changes. No new host frame or ticket is supplied.
        state.value = ready()
        assertEquals(RemoteDecoderStartup.Ready, waiting.await())
        assertEquals(listOf(exactTicket), queued)
        retainedTicket = null
        assertNull(retainedTicket)
    }

    @Test fun invalidatedScopeDuringStartupNeverQueuesRetainedFrame() = runBlocking {
        val state = MutableStateFlow<DecoderStatus>(DecoderStatus.Starting)
        var current = true
        var queued = false
        val waiting = async(start = CoroutineStart.UNDISPATCHED) {
            val result = awaitRemoteDecoderStartup(state, deadline()) { current }
            if (result == RemoteDecoderStartup.Ready) queued = true
            result
        }
        current = false
        state.value = ready()
        assertEquals(RemoteDecoderStartup.Invalidated, waiting.await())
        assertFalse(queued)
    }

    @Test fun cancellationDuringStartupRunsTicketCleanupWithoutQueueing() = runBlocking {
        val state = MutableStateFlow<DecoderStatus>(DecoderStatus.Starting)
        val released = CompletableDeferred<Unit>()
        var queued = false
        val waiting = launch(start = CoroutineStart.UNDISPATCHED) {
            try {
                if (awaitRemoteDecoderStartup(state, deadline()) { true } == RemoteDecoderStartup.Ready) queued = true
            } finally { released.complete(Unit) }
        }
        assertFalse(released.isCompleted)
        waiting.cancel(CancellationException("owned startup cancelled"))
        waiting.join()
        released.await()
        state.value = ready()
        assertFalse(queued)
        assertTrue(waiting.isCancelled)
    }

    @Test fun existingReadyOwnerAdmitsCurrentFrameImmediately() = runBlocking {
        assertEquals(RemoteDecoderStartup.Ready, awaitRemoteDecoderStartup(MutableStateFlow(ready()), deadline()) { true })
    }

    @Test fun alreadyInvalidSurfaceRefusesEvenWithReadyCodec() = runBlocking {
        assertEquals(RemoteDecoderStartup.Invalidated, awaitRemoteDecoderStartup(MutableStateFlow(ready()), deadline()) { false })
    }

    @Test fun recoveryIsNotReadinessAndCannotAdmitFrame() = runBlocking {
        assertEquals(RemoteDecoderStartup.Failed, awaitRemoteDecoderStartup(
            MutableStateFlow<DecoderStatus>(DecoderStatus.RecoveryRequired("SurfaceAbandoned")), deadline()) { true })
    }

    @Test fun retiredOwnerCannotQueueAFrame() = runBlocking {
        assertEquals(RemoteDecoderStartup.Failed, awaitRemoteDecoderStartup(
            MutableStateFlow<DecoderStatus>(DecoderStatus.Retired), deadline()) { true })
    }

    @Test fun expiredAbsoluteStartupDeadlineRefusesWithoutWaiting() = runBlocking {
        assertEquals(RemoteDecoderStartup.Deadline, awaitRemoteDecoderStartup(
            MutableStateFlow<DecoderStatus>(DecoderStatus.Starting), System.nanoTime() - 1) { true })
    }
    @Test fun firstIdrWaitsForFirstSurfaceWithoutAnotherFrame() = runBlocking {
        var ready=false
        val waiting=async(start=CoroutineStart.UNDISPATCHED){awaitRemoteSurfaceStartup(deadline(),{true},{ready})}
        assertFalse(waiting.isCompleted);ready=true
        assertEquals(RemoteDecoderStartup.Ready,waiting.await())
    }
    @Test fun surfaceWaitRejectsRetiredScopeEvenIfSurfaceAppears() = runBlocking {
        var current=true;var ready=false
        val waiting=async(start=CoroutineStart.UNDISPATCHED){awaitRemoteSurfaceStartup(deadline(),{current},{ready})}
        current=false;ready=true
        assertEquals(RemoteDecoderStartup.Invalidated,waiting.await())
    }
    @Test fun surfaceReadinessCannotExtendOriginalStartupDeadline() = runBlocking {
        assertEquals(RemoteDecoderStartup.Deadline,awaitRemoteSurfaceStartup(System.nanoTime()-1,{true},{true}))
    }
    @Test fun cancelledSurfaceWaitPropagatesForExactTicketCleanup() = runBlocking {
        val released=CompletableDeferred<Unit>()
        val waiting=launch(start=CoroutineStart.UNDISPATCHED){try{awaitRemoteSurfaceStartup(deadline(),{true},{false})}finally{released.complete(Unit)}}
        waiting.cancel();waiting.join();assertTrue(released.isCompleted)
    }

}
