package com.visualworkbench.android.remote

import org.junit.Assert.*
import org.junit.Test

class RemoteRenderScheduleTest {
    @Test fun actual_fake_port_gets_first_local_clock_not_media_pts() {
        val clocks=ArrayDeque(listOf(7_000_000_000_000L,7_000_000_000_123L))
        var actualIndex:Int?=null;var scheduled:Long?=null
        val release=releaseRemoteOutputForRender(4,{clocks.removeFirst()}){index,time->actualIndex=index;scheduled=time}
        assertEquals(4,actualIndex);assertEquals(7_000_000_000_000L,scheduled)
        assertEquals(Pair(7_000_000_000_000L,7_000_000_000_123L),release);assertTrue(clocks.isEmpty())
    }
    @Test fun local_request_and_return_remain_distinct() {
        val clocks=ArrayDeque(listOf(200L,250L));var callCount=0
        assertEquals(Pair(200L,250L),releaseRemoteOutputForRender(0,{clocks.removeFirst()}){_,_->callCount++})
        assertEquals(1,callCount)
    }
    @Test fun thrown_release_keeps_exact_primary_and_no_return_clock() {
        val primary=IllegalStateException("fake port failure");var clockCount=0;var caught:Throwable?=null
        try{releaseRemoteOutputForRender(0,{clockCount++;200L}){_,_->throw primary}}catch(error:Throwable){caught=error}
        assertSame(primary,caught);assertEquals(1,clockCount)
    }
    private fun ledger():DecoderTickets {
        val scope=DecoderScope(1uL,"session",1uL,"target",1u)
        val config=DecoderConfig(scope,1uL,800,600,800,600,byteArrayOf(1,2),byteArrayOf(3,4),byteArrayOf(5,6))
        return DecoderTickets(config).also{assertNull(it.admit(DecoderFrame(1uL,scope,1uL,1uL,123,byteArrayOf(1),100,true),100))}
    }
    @Test fun unchanged_exact_pts_and_local_render_chronology_admit_only_owned_ticket() {
        val ledger=ledger();val clocks=ArrayDeque(listOf(200L,210L));var scheduled=0L
        val (request,returned)=releaseRemoteOutputForRender(0,{clocks.removeFirst()}){_,time->scheduled=time}
        assertTrue(ledger.released(123,request,returned));assertEquals(0,ledger.renderIdentity(123,scheduled,230).failureMask)
        assertEquals(1uL,ledger.rendered(123,scheduled,230)?.ticket)
    }
    @Test fun changed_pts_or_pre_request_render_still_refuse_and_retain_ticket() {
        val ledger=ledger();assertTrue(ledger.released(123,200,210))
        assertNull(ledger.rendered(124,220,230));assertNotNull(ledger.outstanding)
        assertNull(ledger.rendered(123,199,230));assertNotNull(ledger.outstanding)
        assertEquals(16,ledger.renderIdentity(123,199,230).failureMask)
    }
}
