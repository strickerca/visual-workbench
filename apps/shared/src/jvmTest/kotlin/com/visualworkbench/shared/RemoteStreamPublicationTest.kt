package com.visualworkbench.shared
import kotlin.test.*
import org.junit.Test
class RemoteStreamPublicationTest {
    private fun state(revision:ULong,status:RemoteEditStatus=RemoteEditStatus.Viewing)=RemoteEditState(revision,status,null,null,null,null,null)
    @Test fun delayedWatcherCannotRollBackFrameSideRevocation(){
        val published=mutableListOf<RemoteEditState>();val owner=RemoteStreamPublication(state(0uL)){published+=it}
        assertTrue(owner.offer(state(4uL,RemoteEditStatus.Paused)))
        assertFalse(owner.offer(state(3uL)));assertFalse(owner.offer(state(4uL)))
        assertEquals(listOf(state(4uL,RemoteEditStatus.Paused)),published)
    }
    @Test fun firstFrameSnapshotIsPublishedSynchronously(){
        var seen=state(0uL);val owner=RemoteStreamPublication(seen){seen=it}
        assertTrue(owner.offer(state(2uL)));assertEquals(2uL,seen.revision)
    }
    @Test fun delayedWatcherCannotCancelSamplesAdmittedAfterSynchronousPublication(){
        val consumers=RemoteConsumerRegistry();var queuedNewGrantSamples=0;var invalidations=0
        consumers.register(object:RemoteConsumerOwner{
            override fun invalidate(){invalidations++;queuedNewGrantSamples=0}
            override suspend fun retire():Boolean=true
        })
        var published=state(0uL)
        val publication=RemoteStreamPublication(published){next->consumers.invalidate();published=next}
        assertTrue(publication.offer(state(3uL,RemoteEditStatus.Controlling)))
        assertEquals(3uL,published.revision)
        queuedNewGrantSamples=1 // actual consumer work begins after native publication
        assertFalse(publication.offer(state(2uL)))
        assertEquals(1,queuedNewGrantSamples);assertEquals(1,invalidations)
    }
    @Test fun numericCountersHaveOnlyExplicitKeys(){
        val counters=RemoteStreamCounters("received","discarded");counters.increment("received")
        assertEquals(mapOf("received" to 1uL,"discarded" to 0uL),counters.snapshot())
        assertFailsWith<IllegalArgumentException>{counters.increment("unbounded")}
    }
}
