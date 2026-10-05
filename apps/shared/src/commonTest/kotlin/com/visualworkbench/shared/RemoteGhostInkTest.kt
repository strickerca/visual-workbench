package com.visualworkbench.shared

import kotlin.test.*

class RemoteGhostInkTest {
    private fun binding() = RemoteTargetBinding(1uL, "capture", 1uL, "target", 1u, "input")
    private fun segment(seq: ULong = 1uL, created: Long = 0) = GhostSegment(binding(), seq, seq, created,
        listOf(GhostPoint(1f, 2f, 0.5f, created, false)))
    private fun ink(): RemoteGhostInk = RemoteGhostInk().also { assertTrue(it.activate(binding(), 0)) }

    @Test fun missing_callback_still_expires_at_500_ms() {
        val ink = ink(); assertTrue(ink.add(segment()))
        assertEquals(1, ink.snapshot(499_999_999).size)
        assertTrue(ink.snapshot(500_000_000).isEmpty())
    }
    @Test fun ack_fades_at_150_ms_and_duplicate_never_restarts_fade() {
        val ink = ink(); ink.add(segment())
        assertTrue(ink.acknowledge(binding(), 1uL, 1uL, 8uL, 100_000_000))
        assertFalse(ink.acknowledge(binding(), 1uL, 1uL, 8uL, 150_000_000))
        assertEquals(0.5f, ink.snapshot(175_000_000).single().alpha)
        assertTrue(ink.snapshot(250_000_000).isEmpty())
    }
    @Test fun later_distinct_covering_ack_preserves_first_fade_origin() {
        val ink = ink(); assertTrue(ink.add(segment()))
        assertTrue(ink.acknowledge(binding(), 1uL, 1uL, 8uL, 100_000_000))
        assertTrue(ink.acknowledge(binding(), 1uL, 2uL, 9uL, 200_000_000))
        val visible = ink.snapshot(200_000_000).single()
        assertEquals(100_000_000L, visible.acknowledgedLocalNanos)
        assertEquals(1f / 3f, visible.alpha, 0.00001f)
        assertTrue(ink.snapshot(250_000_000).isEmpty())
    }
    @Test fun late_ack_is_clamped_by_independent_creation_expiry() {
        val ink = ink(); ink.add(segment())
        assertTrue(ink.acknowledge(binding(), 1uL, 1uL, 1uL, 490_000_000))
        assertTrue(ink.snapshot(500_000_000).isEmpty())
    }
    @Test fun delayed_callback_can_arrive_after_overlay_tick_without_clock_reversal() {
        val ink = ink(); ink.add(segment()); ink.snapshot(200_000_000)
        assertTrue(ink.acknowledge(binding(), 1uL, 1uL, 1uL, 100_000_000))
        assertEquals(1f / 3f, ink.snapshot(200_000_000).single().alpha, 0.00001f)
        assertTrue(ink.snapshot(250_000_000).isEmpty())
    }
    @Test fun predictions_do_not_advance_sequence_or_creation_expiry() {
        val ink = ink(); val first = segment(); ink.add(first)
        val prediction = GhostPoint(3f, 4f, 0.5f, 50_000_000, true)
        assertTrue(ink.update(first.copy(points = first.points + prediction)))
        assertFalse(ink.update(first.copy(lastInputSequence = 2uL, points = first.points + prediction)))
        assertEquals(1uL, ink.snapshot(20_000_000).single().segment.lastInputSequence)
        assertTrue(ink.snapshot(500_000_000).isEmpty())
    }
    @Test fun appended_actual_sample_needs_new_ack_and_never_extends_expiry() {
        val ink = ink(); val first = segment(); ink.add(first)
        ink.acknowledge(binding(), 1uL, 1uL, 1uL, 10_000_000)
        assertTrue(ink.update(first.copy(lastInputSequence = 2uL,
            points = first.points + GhostPoint(3f, 4f, 0.5f, 20_000_000, false))))
        assertEquals(1f, ink.snapshot(200_000_000).single().alpha)
        assertTrue(ink.snapshot(500_000_000).isEmpty())
    }
    @Test fun full_scope_and_grant_retirement_clear_immediately() {
        val ink = ink(); ink.add(segment()); ink.retire(binding())
        assertTrue(ink.snapshot(1).isEmpty()); assertFalse(ink.add(segment()))
        val next = binding().copy(connectionEpoch = 2uL, inputSessionId = "next")
        ink.activate(next, 2); assertTrue(ink.add(segment(created = 2).copy(binding = next)))
        ink.retire(binding()); assertEquals(1, ink.snapshot(3).size)
        assertFalse(ink.acknowledge(binding(), 5uL, 5uL, 5uL, 3))
    }
    @Test fun stale_frame_sequence_and_other_geometry_cannot_ack() {
        val ink = ink(); ink.add(segment()); ink.acknowledge(binding(), 1uL, 2uL, 2uL, 10)
        assertFalse(ink.acknowledge(binding(), 0uL, 3uL, 3uL, 20))
        assertFalse(ink.acknowledge(binding(), 2uL, 1uL, 4uL, 20))
        assertFalse(ink.acknowledge(binding().copy(geometryRevision = 2u), 2uL, 3uL, 3uL, 20))
    }
    @Test fun limits_nan_and_mutable_caller_lists_are_bounded() {
        val ink = ink(); val points = mutableListOf(GhostPoint(1f, 1f, 1f, 0, false))
        assertTrue(ink.add(segment().copy(points = points))); points.clear()
        assertEquals(1, ink.snapshot(1).single().segment.points.size)
        try {
            (ink.snapshot(1).single().segment.points as? MutableList<GhostPoint>)?.clear()
        } catch (_: UnsupportedOperationException) {
            // An external read-only singleton copy may reject mutation.
        }
        assertEquals(1, ink.snapshot(1).single().segment.points.size)
        assertFalse(ink.add(segment(2uL).copy(points = listOf(GhostPoint(Float.NaN, 0f, 1f, 0, false)))))
        assertFalse(ink.add(segment(2uL).copy(points = List(257) { GhostPoint(0f, 0f, 1f, 0, false) })))
        for (seq in 2..64) assertTrue(ink.add(segment(seq.toULong())))
        assertFalse(ink.add(segment(65uL)))
    }
    @Test fun no_predicted_only_segment_or_expiry_reset_on_update() {
        val ink = ink(); val first = segment(); ink.add(first)
        assertFalse(ink.add(segment(2uL).copy(points = listOf(GhostPoint(0f, 0f, 1f, 0, true)))))
        assertFalse(ink.update(first.copy(createdLocalNanos = 100)))
        assertTrue(ink.activate(binding(), 100)); assertTrue(ink.snapshot(500_000_000).isEmpty())
    }
    @Test fun long_stroke_new_segment_anchor_never_invents_sequence_or_extends_old_entry() {
        val ink = ink(); val old = segment(); ink.add(old)
        val anchor = old.points.single().copy(anchor = true)
        val fresh = segment(2uL, 400_000_000).copy(points = listOf(anchor,
            GhostPoint(4f, 5f, 0.6f, 400_000_000, false)))
        assertTrue(ink.add(fresh))
        val live = ink.snapshot(500_000_000)
        assertEquals(1, live.size); assertEquals(2uL, live.single().segment.firstInputSequence)
        assertEquals(2uL, live.single().segment.lastInputSequence)
        assertTrue(ink.snapshot(900_000_000).isEmpty())
        assertFalse(ink.add(segment(3uL, 900_000_000).copy(points = listOf(anchor))))
    }
}
