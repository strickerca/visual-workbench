package com.visualworkbench.shared

import kotlin.test.*

class RemoteEditCompositionTest {
    private fun binding(input:String="input")=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,input)
    private fun admission(seq:ULong,time:Long,input:String="input")=RemoteInputAdmission(binding(input),seq,time)
    @Test fun visible_letterbox_excludes_padding_and_right_bottom_edges(){
        val view=assertNotNull(RemoteViewport.fit(1000,1000,639,479));val rect=RemotePhysicalRect(-400,-200,639u,479u)
        assertNull(view.host(view.left-1f,view.top,rect));assertNull(view.host(view.left+view.width,view.top,rect));assertNull(view.host(view.left,view.top+view.height,rect))
        assertEquals(-400 to -200,view.host(view.left,view.top,rect));val last=assertNotNull(view.host(view.left+view.width-0.01f,view.top+view.height-0.01f,rect));assertTrue(last.first<239&&last.second<279)
    }
    @Test fun viewport_rejects_nonfinite_overflow_and_missing_physical_bounds(){
        val view=assertNotNull(RemoteViewport.fit(640,480,640,480));assertNull(view.host(Float.NaN,1f,RemotePhysicalRect(0,0,640u,480u)))
        assertNull(view.host(10f,1f,RemotePhysicalRect(Int.MAX_VALUE,0,640u,480u)));assertNull(RemoteViewport.fit(0,480,640,480));assertNull(RemoteViewport.fit(640,480,4097,480))
    }
    @Test fun multi_second_contact_keeps_fresh_segments_without_resetting_old_creation(){
        val ink=RemoteGhostInk();val segments=RemoteGhostSegments(ink)
        for(n in 1..250){val t=(n-1)*10_000_000L;assertTrue(segments.admitted(admission(n.toULong(),t),GhostPoint(n.toFloat(),1f,0.5f,t,false),true))}
        val visible=ink.snapshot(2_490_000_000L);assertTrue(visible.isNotEmpty());assertTrue(visible.all{2_490_000_000L-it.segment.createdLocalNanos<500_000_000L})
        assertTrue(visible.any{it.segment.points.first().anchor});assertEquals(250uL,visible.maxOf{it.segment.lastInputSequence})
    }
    @Test fun sample_count_rollover_has_one_anchor_and_only_new_actual_sequences(){
        val ink=RemoteGhostInk();val segments=RemoteGhostSegments(ink)
        for(n in 1..70){val t=n*1_000L;assertTrue(segments.admitted(admission(n.toULong(),t),GhostPoint(n.toFloat(),1f,0.5f,t,false),true))}
        val entries=ink.snapshot(70_000);assertEquals(2,entries.size);val next=entries.last().segment
        assertEquals(64uL,next.firstInputSequence);assertEquals(70uL,next.lastInputSequence);assertTrue(next.points.first().anchor);assertEquals(7,next.points.count{!it.anchor&&!it.predicted})
    }
    @Test fun predicted_points_never_advance_admission_or_refresh_creation_deadline(){
        val ink=RemoteGhostInk();val segments=RemoteGhostSegments(ink);val point=GhostPoint(1f,1f,0.5f,1_000,false)
        assertTrue(segments.admitted(admission(1uL,1_000),point,true));assertTrue(segments.prediction(point.copy(x=2f,localNanos=12_001_000,predicted=true)))
        val entry=ink.snapshot(1_000).single().segment;assertEquals(1uL,entry.lastInputSequence);assertEquals(1_000L,entry.createdLocalNanos)
        assertFalse(segments.admitted(admission(2uL,2_000),point.copy(localNanos=2_000,predicted=true),true));assertTrue(ink.snapshot(500_001_000).isEmpty())
    }
    @Test fun full_input_binding_change_and_retirement_clear_old_ghosts(){
        val ink=RemoteGhostInk();val segments=RemoteGhostSegments(ink)
        assertTrue(segments.admitted(admission(1uL,1_000),GhostPoint(1f,1f,0.5f,1_000,false),true))
        assertTrue(segments.admitted(admission(1uL,2_000,"fresh-input"),GhostPoint(2f,1f,0.5f,2_000,false),true))
        assertEquals(binding("fresh-input"),ink.snapshot(2_000).single().segment.binding);segments.retire();assertTrue(ink.snapshot(2_001).isEmpty())
    }
    @Test fun only_matching_native_ack_covers_actual_samples_and_records_callback_timing(){
        val ledger=RemoteEchoLedger();assertTrue(ledger.record(admission(7uL,10_000_000)))
        assertFalse(ledger.acknowledge(RemoteRenderedAcknowledgment(binding("other"),1uL,8uL,7uL),30_000_000))
        assertTrue(ledger.acknowledge(RemoteRenderedAcknowledgment(binding(),1uL,8uL,7uL),30_000_000))
        val timing=ledger.snapshot(30_000_000);assertEquals(1uL,timing.observed);assertEquals(20_000_000L,timing.p95Nanos);assertEquals(0,timing.pending)
        assertFalse(ledger.acknowledge(RemoteRenderedAcknowledgment(binding(),1uL,8uL,7uL),40_000_000))
    }
    @Test fun missing_callbacks_and_retirement_have_separate_honest_coverage(){
        val ledger=RemoteEchoLedger();ledger.record(admission(1uL,0));ledger.record(admission(2uL,10_000_000))
        val first=ledger.snapshot(500_000_000);assertEquals(1uL,first.missing);assertEquals(0uL,first.observed);assertEquals(1,first.pending);assertNull(first.p95Nanos)
        ledger.retire();assertEquals(2uL,ledger.snapshot(500_000_001).missing)
    }
}
