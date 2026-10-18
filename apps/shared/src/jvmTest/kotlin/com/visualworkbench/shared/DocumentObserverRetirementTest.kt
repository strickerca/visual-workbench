package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.collect
import org.junit.Assert.*
import org.junit.Test

class DocumentObserverRetirementTest {
    @Test fun cancellationDuringTheFirstJoinWaitsForOldCleanupThenRestoresExactlyOneObserver() = runBlocking {
        val cleanupEntered=CompletableDeferred<Unit>();val cleanupRelease=CompletableDeferred<Unit>()
        val scope=CoroutineScope(coroutineContext+SupervisorJob())
        val events=MutableSharedFlow<Unit>()
        var active=1;var restores=0;var adopted=false;var replacement:Job?=null
        var old:Job?=scope.launch(start=CoroutineStart.UNDISPATCHED){
            try{events.collect{}}finally{withContext(NonCancellable){cleanupEntered.complete(Unit);cleanupRelease.await();active--}}
        }
        val switch=launch(start=CoroutineStart.UNDISPATCHED){
            withDocumentObserversRetired(
                retire={old?.cancelAndJoin();old=null},
                restore={assertEquals(0,active);active++;restores++;replacement=scope.launch(start=CoroutineStart.UNDISPATCHED){try{events.collect{}}finally{active--}}},
                adopt={adopted=true},
            )
        }
        try {
            cleanupEntered.await();switch.cancel();yield()
            assertEquals(1,active);assertEquals(0,restores);assertFalse(switch.isCompleted)
            cleanupRelease.complete(Unit);withTimeout(3000){switch.join()}
            assertFalse(adopted);assertEquals(1,restores);assertEquals(1,active);assertEquals(1,events.subscriptionCount.value)
        } finally {cleanupRelease.complete(Unit);withContext(NonCancellable){switch.cancelAndJoin();old?.cancelAndJoin();replacement?.cancelAndJoin();scope.coroutineContext[Job]?.cancelAndJoin()}}
    }

    @Test fun cancellationDuringAdoptionRestoresOnceAfterConfirmedOldRetirement()=runBlocking {
        val entered=CompletableDeferred<Unit>();var active=1;var restores=0
        val switch=launch(start=CoroutineStart.UNDISPATCHED){
            withDocumentObserversRetired(retire={active=0},restore={active++;restores++}){
                entered.complete(Unit);awaitCancellation()
            }
        }
        entered.await();switch.cancelAndJoin()
        assertEquals(1,restores);assertEquals(1,active)
    }

    @Test fun failedRetirementCannotRestoreAnObserverBesideAnUnknownOwner()=runBlocking {
        var attempts=0;var restored=false;var adopted=false
        try {
            withDocumentObserversRetired(retire={attempts++;throw IllegalStateException("retained owner")},restore={restored=true}){adopted=true}
            fail("retirement must fail closed")
        }catch(_:IllegalStateException){}
        assertEquals(2,attempts);assertFalse(restored);assertFalse(adopted)
    }

    @Test fun closedProjectGuardDoesNotRecreateObserversAfterCancelledRetirement()=runBlocking {
        var current=true;var restores=0;var retirements=0
        val entered=CompletableDeferred<Unit>()
        val switch=launch(start=CoroutineStart.UNDISPATCHED){
            withDocumentObserversRetired(retire={retirements++},restore={if(current)restores++}){entered.complete(Unit);awaitCancellation()}
        }
        entered.await();current=false;switch.cancelAndJoin()
        assertEquals(2,retirements);assertEquals(0,restores)
    }
}
