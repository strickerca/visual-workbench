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

}
