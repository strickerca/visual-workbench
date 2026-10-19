package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

public class GiantTileDeliveryOwnershipTest {
    private class Source
    private class Lease

    @Test public fun failedBulkLiftRetiresKnownSourceThenPrefetchedLease() {
        val events=mutableListOf<String>()
        val source=Source();val lease=Lease()
        val owner=GiantTileDeliveryOwner(source,
            publicationLease={value:Source->assertSame(source,value);events+="prefetch";lease},
            takePixels={_:Source->events+="bulk";error("synthetic ByteArray lift failure")},
            retireSource={value:Source->assertSame(source,value);events+="source"},
            retireLease={value:Lease->assertSame(lease,value);events+="lease"})
        try {owner.take();fail("expected lift failure")}catch(_:IllegalStateException) { }
        owner.release();owner.release()
        assertEquals(listOf("prefetch","bulk","source","lease"),events)
    }

    @Test public fun failedSourceAndLeaseRetirementKeepExactOwnersForRetry() {
        val events=mutableListOf<String>()
        val source=Source();val lease=Lease()
        var sourceFails=true;var leaseFails=true
        val owner=GiantTileDeliveryOwner(source,
            publicationLease={_:Source->lease},
            takePixels={_:Source->byteArrayOf(1,2,3,4)},
            retireSource={value:Source->assertSame(source,value);events+="source";
                if(sourceFails){sourceFails=false;error("source close failed")}},
            retireLease={value:Lease->assertSame(lease,value);events+="lease";
                if(leaseFails){leaseFails=false;error("lease close failed")}})
        assertArrayEquals(byteArrayOf(1,2,3,4),owner.take())
        try {owner.release();fail("expected source error")}catch(_:IllegalStateException) { }
        assertEquals(listOf("source"),events)
        try {owner.release();fail("expected lease error")}catch(_:IllegalStateException) { }
        assertEquals(listOf("source","source","lease"),events)
        owner.release();owner.release()
        assertEquals(listOf("source","source","lease","lease"),events)
    }

    @Test public fun undeliveredLateSourceNeedsNoBulkLiftOrForeignLease() {
        val events=mutableListOf<String>()
        val owner=GiantTileDeliveryOwner(Source(),
            publicationLease={_:Source->events+="prefetch";Lease()},
            takePixels={_:Source->events+="bulk";byteArrayOf(0)},
            retireSource={_:Source->events+="source"},
            retireLease={_:Lease->events+="lease"})
        owner.release()
        assertEquals(listOf("source"),events)
        try {owner.take();fail("closed source accepted")}catch(error:GiantImageFailure) {
            assertEquals(GiantImageFailureKind.Closed,error.kind)
        }
    }
}
