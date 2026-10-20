package com.visualworkbench.android.editor

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class CanvasRetirementOwnerTest {
    @Test fun detachedRendererBlocksSuccessorUntilActualConfirmation() {
        val owner=CanvasRetirementOwner();val old:suspend()->Unit={};val next:suspend()->Unit={};var wakes=0
        owner.attach(old){};assertTrue(owner.canCreate(old));assertTrue(owner.detach(old))
        owner.attach(next){wakes++};assertFalse(owner.canCreate(next));assertFalse(owner.detach(old))
        owner.confirmed(old);assertTrue(owner.canCreate(next));assertEquals(1,wakes)
        owner.confirmed(next);assertEquals(1,owner.retainedCount())
    }
    @Test fun aggregateRetirementJoinsDetachedRendererBeforeReturning()=runBlocking {
        val owner=CanvasRetirementOwner();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        val old:suspend()->Unit={entered.complete(Unit);release.await()};val next:suspend()->Unit={}
        owner.attach(old){};owner.detach(old);owner.attach(next){}
        val retiring=launch(start=CoroutineStart.UNDISPATCHED){owner.retireAll()};entered.await()
        assertFalse(retiring.isCompleted);assertFalse(owner.canCreate(next))
        release.complete(Unit);retiring.join();assertFalse(owner.canCreate(next))
        owner.confirmed(old);assertTrue(owner.canCreate(next))
    }
    @Test fun failedDetachedRetirementKeepsItsOwnershipAndBlocksAllocation()=runBlocking {
        val owner=CanvasRetirementOwner();val old:suspend()->Unit={throw IllegalStateException("renderer retained")};val next:suspend()->Unit={}
        owner.attach(old){};owner.detach(old);owner.attach(next){}
        assertTrue(runCatching{owner.retireAll()}.isFailure)
        assertEquals(2,owner.retainedCount());assertFalse(owner.canCreate(next))
    }
    @Test fun surfaceAttachedDuringAggregateReleaseIsGatedAndJoined()=runBlocking {
        val owner=CanvasRetirementOwner();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();var nextJoined=false
        val old:suspend()->Unit={entered.complete(Unit);release.await()}
        val next:suspend()->Unit={nextJoined=true}
        owner.attach(old){}
        val retiring=launch(start=CoroutineStart.UNDISPATCHED){owner.retireAll()};entered.await()
        owner.detach(old);owner.attach(next){};owner.confirmed(old)
        assertFalse(owner.canCreate(next));assertFalse(nextJoined)
        release.complete(Unit);retiring.join();assertTrue(nextJoined);assertTrue(owner.canCreate(next))
    }

    @Test fun aggregateObservesTheAlreadyPendingReleaseOfItsExactDetachedHook()=runBlocking {
        withTimeout(2000) {
            val owner=CanvasRetirementOwner()
            val oldProgress=CanvasRetirementProgress()
            val oldRelease=oldProgress.begin(CanvasRetirementReason.SurfaceChanged)
            val pending=oldProgress.current
            val nextProgress=CanvasRetirementProgress()
            nextProgress.begin(CanvasRetirementReason.ProjectRetirement)
            val oldEntered=CompletableDeferred<Unit>();val finishOld=CompletableDeferred<Unit>()
            val nextEntered=CompletableDeferred<Unit>();val finishNext=CompletableDeferred<Unit>()
            val old:suspend()->Unit={oldEntered.complete(Unit);finishOld.await()}
            val next:suspend()->Unit={nextEntered.complete(Unit);finishNext.await()}
            owner.attach(old,{}, {oldProgress.current});owner.detach(old)
            owner.attach(next,{}, {nextProgress.current})
            val work=launch(start=CoroutineStart.UNDISPATCHED){owner.retireAll()}
            try {
                oldEntered.await()
                assertFalse(work.isCompleted)
                assertEquals(pending,owner.progress)
                assertEquals(CanvasRetirementReason.SurfaceChanged,owner.progress?.reason)
                // The library callback may precede its Main delivery; querying
                // the aggregate observes this same already pending generation.
                withContext(Dispatchers.Default){oldRelease.reached(CanvasRetirementMilestone.LibraryCallbackEntered)}
                assertEquals(pending?.generation,owner.progress?.generation)
                assertEquals(CanvasRetirementMilestone.LibraryCallbackEntered,owner.progress?.milestone)
                oldRelease.reached(CanvasRetirementMilestone.CompletionPublished)
                finishOld.complete(Unit);nextEntered.await()
                assertFalse(work.isCompleted)
                assertEquals(nextProgress.current,owner.progress)
                owner.confirmed(old)
                assertEquals(nextProgress.current,owner.progress)
                finishNext.complete(Unit);work.join()
                assertEquals(1,owner.retainedCount())
            } finally {
                finishOld.complete(Unit);finishNext.complete(Unit)
                withContext(NonCancellable){work.join()}
            }
        }
    }

    @Test fun anUnobservedAwaitedHookCannotBorrowAnotherHooksProgress()=runBlocking {
        withTimeout(2000) {
            val owner=CanvasRetirementOwner()
            val nextProgress=CanvasRetirementProgress()
            nextProgress.begin(CanvasRetirementReason.ProjectRetirement)
            val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
            val old:suspend()->Unit={entered.complete(Unit);release.await()}
            val next:suspend()->Unit={}
            owner.attach(old){};owner.detach(old);owner.attach(next,{}, {nextProgress.current})
            val work=launch(start=CoroutineStart.UNDISPATCHED){owner.retireAll()}
            try {
                entered.await()
                assertNull(owner.progress)
                assertFalse(work.isCompleted)
            } finally {release.complete(Unit);withContext(NonCancellable){work.join()}}
        }
    }

    @Test fun failedAggregateKeepsItsExactFailureAndHookObservation()=runBlocking {
        withTimeout(2000) {
            val owner=CanvasRetirementOwner()
            val oldProgress=CanvasRetirementProgress()
            val oldRelease=oldProgress.begin(CanvasRetirementReason.WindowDetached)
            val failure=IllegalStateException("synthetic retained renderer")
            val old:suspend()->Unit={oldRelease.failed(CanvasRetirementMilestone.ReaderCloseRunning);throw failure}
            val nextProgress=CanvasRetirementProgress()
            nextProgress.begin(CanvasRetirementReason.ProjectRetirement)
            val next:suspend()->Unit={}
            owner.attach(old,{}, {oldProgress.current});owner.detach(old);owner.attach(next,{}, {nextProgress.current})
            assertSame(failure,runCatching{owner.retireAll()}.exceptionOrNull())
            assertEquals(oldProgress.current,owner.progress)
            assertSame(failure,runCatching{owner.retireAll()}.exceptionOrNull())
            assertEquals(2,owner.retainedCount())
            assertFalse(owner.canCreate(next))
        }
    }

    @Test fun aggregateCancellationWaitsItsExistingCleanupAndKeepsTheAwaitedHook()=runBlocking {
        withTimeout(2000) {
            val owner=CanvasRetirementOwner()
            val progress=CanvasRetirementProgress()
            progress.begin(CanvasRetirementReason.ProjectRetirement)
            val entered=CompletableDeferred<Unit>();val cleanup=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
            val cancellation=CancellationException("synthetic aggregate cancellation")
            var caught:Throwable?=null
            val hook:suspend()->Unit={
                try{entered.complete(Unit);awaitCancellation()}
                finally{cleanup.complete(Unit);withContext(NonCancellable){release.await()}}
            }
            owner.attach(hook,{}, {progress.current})
            val work=launch(start=CoroutineStart.UNDISPATCHED){
                try{owner.retireAll()}catch(error:Throwable){caught=error;throw error}
            }
            try {
                entered.await();work.cancel(cancellation);cleanup.await()
                assertFalse(work.isCompleted)
                assertEquals(progress.current,owner.progress)
                release.complete(Unit);work.join()
                assertTrue(caught===cancellation||caught?.cause===cancellation)
                assertEquals(progress.current,owner.progress)
                assertEquals(1,owner.retainedCount())
                assertFalse(owner.canCreate(hook))
            } finally {release.complete(Unit);withContext(NonCancellable){work.join()}}
        }
    }

    @Test fun observerFailureCannotChangeAnAwaitedJoinOrItsActualFailure()=runBlocking {
        withTimeout(2000) {
            val owner=CanvasRetirementOwner()
            val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
            val failure=IllegalStateException("synthetic actual release failure")
            val observerFailure=IllegalArgumentException("synthetic observation failure")
            val hook:suspend()->Unit={entered.complete(Unit);release.await();throw failure}
            owner.attach(hook,{}, {throw observerFailure})
            var caught:Throwable?=null
            val work=launch(start=CoroutineStart.UNDISPATCHED){
                caught=runCatching{owner.retireAll()}.exceptionOrNull()
            }
            try {
                entered.await()
                assertNull(owner.progress)
                assertFalse(work.isCompleted)
                assertEquals(1,owner.retainedCount())
                assertFalse(owner.canCreate(hook))
                release.complete(Unit);work.join()
                assertTrue(caught===failure||caught?.cause===failure)
                assertNull(owner.progress)
                assertSame(failure,runCatching{owner.retireAll()}.exceptionOrNull())
                assertFalse(owner.canCreate(hook))
            } finally {release.complete(Unit);withContext(NonCancellable){work.join()}}
        }
    }

}
