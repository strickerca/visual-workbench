package com.visualworkbench.android.remote

import org.junit.Assert.*
import org.junit.Test

class DecoderRenderIdentityTest {
    private val scope=DecoderScope(1uL,"session",1uL,"target",1u)
    private val config=DecoderConfig(scope,1uL,800,600,800,600,byteArrayOf(1,2),byteArrayOf(3,4),byteArrayOf(5,6))
    private fun tickets():DecoderTickets=DecoderTickets(config).also {
        assertNull(it.admit(DecoderFrame(1uL,scope,1uL,1uL,10,byteArrayOf(1),100,true),100))
        assertTrue(it.released(10,200,210))
    }
    private fun refused(pts:Long,rendered:Long,callback:Long,bit:Int) {
        val owner=tickets();val diagnostic=owner.renderIdentity(pts,rendered,callback)
        assertTrue(diagnostic.failureMask and bit != 0)
        assertNull(owner.rendered(pts,rendered,callback));assertNotNull(owner.outstanding)
    }
    @Test fun current_exact_callback_diagnostic_zero_then_consumes_only_that_ticket(){
        val owner=tickets();assertEquals(0,owner.renderIdentity(10,220,230).failureMask)
        assertNotNull(owner.outstanding);assertEquals(1uL,owner.rendered(10,220,230)?.ticket);assertNull(owner.outstanding)
    }
    @Test fun missing_ticket_and_release_facts_remain_explicit(){
        val owner=DecoderTickets(config);assertEquals(7,owner.renderIdentity(10,200,210).failureMask)
        assertNull(owner.rendered(10,200,210))
    }
    @Test fun missing_output_release_never_consumes_ticket(){
        val owner=DecoderTickets(config);assertNull(owner.admit(DecoderFrame(1uL,scope,1uL,1uL,10,byteArrayOf(1),100,true),100))
        assertEquals(6,owner.renderIdentity(10,200,210).failureMask);assertNull(owner.rendered(10,200,210));assertNotNull(owner.outstanding)
    }
    @Test fun changed_pts_is_distinct_and_keeps_ledger(){refused(11,220,230,8)}
    @Test fun render_before_actual_release_request_is_distinct(){refused(10,199,230,16)}
    @Test fun callback_before_render_is_distinct(){refused(10,240,230,32)}
    @Test fun callback_before_enqueue_is_distinct(){refused(10,220,99,64)}
    @Test fun original_500ms_age_is_distinct_and_not_extended(){refused(10,220,100+FRAME_DEADLINE_NANOS,128)}
    @Test fun relative_deltas_are_bounded_without_absolute_clock_or_ids(){
        val diagnostic=tickets().renderIdentity(Long.MAX_VALUE,Long.MIN_VALUE,Long.MAX_VALUE)
        assertEquals(1_000_000_000_000L,diagnostic.ptsDeltaUs)
        assertEquals(-1_000_000_000_000L,diagnostic.renderedFromRequestNanos)
        assertEquals(1_000_000_000_000L,diagnostic.callbackFromRenderedNanos)
        assertTrue(diagnostic.boundedText().length<256)
        assertFalse(diagnostic.boundedText().contains("session"));assertFalse(diagnostic.boundedText().contains("target"))
    }
}
