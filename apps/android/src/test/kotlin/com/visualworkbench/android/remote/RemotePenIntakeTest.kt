package com.visualworkbench.android.remote

import org.junit.Assert.*
import org.junit.Test

class RemotePenIntakeTest {
    /** Records the calls an intake makes, in order, as the controller would see them. */
    private class Calls(private val busyAt: Int = -1, private val fullAfter: Int = Int.MAX_VALUE) {
        val log = mutableListOf<String>()
        private var reservations = 0
        private var queued = 0
        val reserve: (UInt) -> Boolean = { count -> log += "reserve:$count"; reservations++ != busyAt }
        val enqueue: (Int) -> Boolean = { sample -> log += "enqueue:$sample"; queued++ < fullAfter }
    }

    @Test fun switched_off_makes_the_same_calls_in_the_same_order_as_before() {
        val intake = RemotePenIntake(longBatches = false)
        assertEquals(7, intake.maxHistory); assertEquals(8, intake.queueCapacity)
        // One reservation for the whole event, then every sample in order.
        for (size in 1..8) {
            val calls = Calls()
            assertEquals(RemotePenIntake.Result.Admitted, intake.admit((1..size).toList(), calls.reserve, calls.enqueue))
            assertEquals(listOf("reserve:$size") + (1..size).map { "enqueue:$it" }, calls.log)
        }
        // A busy editor command refuses the event before anything is queued.
        val busy = Calls(busyAt = 0)
        assertEquals(RemotePenIntake.Result.CommandBusy, intake.admit(listOf(1, 2, 3), busy.reserve, busy.enqueue))
        assertEquals(listOf("reserve:3"), busy.log)
        // A full queue stops at the first refused sample, exactly like `all {}`.
        val full = Calls(fullAfter = 2)
        assertEquals(RemotePenIntake.Result.Refused, intake.admit(listOf(1, 2, 3, 4), full.reserve, full.enqueue))
        assertEquals(listOf("reserve:4", "enqueue:1", "enqueue:2", "enqueue:3"), full.log)
    }

    @Test fun switched_on_reserves_a_long_event_in_groups_of_eight_and_keeps_sample_order() {
        val intake = RemotePenIntake(longBatches = true)
        assertEquals(15, intake.maxHistory); assertEquals(32, intake.queueCapacity)
        val short = Calls()
        assertEquals(RemotePenIntake.Result.Admitted, intake.admit((1..8).toList(), short.reserve, short.enqueue))
        assertEquals(listOf("reserve:8") + (1..8).map { "enqueue:$it" }, short.log)
        val twelve = Calls()
        assertEquals(RemotePenIntake.Result.Admitted, intake.admit((1..12).toList(), twelve.reserve, twelve.enqueue))
        assertEquals(listOf("reserve:8", "reserve:4") + (1..12).map { "enqueue:$it" }, twelve.log)
        val sixteen = Calls()
        assertEquals(RemotePenIntake.Result.Admitted, intake.admit((1..16).toList(), sixteen.reserve, sixteen.enqueue))
        assertEquals(listOf("reserve:8", "reserve:8") + (1..16).map { "enqueue:$it" }, sixteen.log)
    }

    @Test fun the_typed_switch_alone_changes_no_bound_and_with_long_batches_reserves_a_whole_event_at_once() {
        val typedOnly = RemotePenIntake(longBatches = false, typed = true)
        assertEquals(7, typedOnly.maxHistory); assertEquals(8, typedOnly.maxSamples); assertEquals(8, typedOnly.queueCapacity)
        val eight = Calls()
        assertEquals(RemotePenIntake.Result.Admitted, typedOnly.admit((1..8).toList(), eight.reserve, eight.enqueue))
        assertEquals(listOf("reserve:8") + (1..8).map { "enqueue:$it" }, eight.log)
        val both = RemotePenIntake(longBatches = true, typed = true)
        assertEquals(31, both.maxHistory); assertEquals(32, both.maxSamples); assertEquals(64, both.queueCapacity)
        val twenty = Calls()
        assertEquals(RemotePenIntake.Result.Admitted, both.admit((1..20).toList(), twenty.reserve, twenty.enqueue))
        assertEquals(listOf("reserve:20") + (1..20).map { "enqueue:$it" }, twenty.log)
        val full = Calls()
        assertEquals(RemotePenIntake.Result.Admitted, both.admit((1..32).toList(), full.reserve, full.enqueue))
        assertEquals("reserve:32", full.log.first()); assertEquals(33, full.log.size)
        val busy = Calls(busyAt = 0)
        assertEquals(RemotePenIntake.Result.CommandBusy, both.admit((1..20).toList(), busy.reserve, busy.enqueue))
        assertEquals(listOf("reserve:20"), busy.log)
    }

    @Test fun switched_on_never_queues_a_sample_of_an_event_it_could_not_fully_reserve() {
        val intake = RemotePenIntake(longBatches = true)
        val busy = Calls(busyAt = 0)
        assertEquals(RemotePenIntake.Result.CommandBusy, intake.admit((1..12).toList(), busy.reserve, busy.enqueue))
        assertEquals(listOf("reserve:8"), busy.log)
        // A refusal after the first group is not "busy": the caller must pause,
        // which retires the group already reserved.
        val late = Calls(busyAt = 1)
        assertEquals(RemotePenIntake.Result.Refused, intake.admit((1..12).toList(), late.reserve, late.enqueue))
        assertEquals(listOf("reserve:8", "reserve:4"), late.log)
        val full = Calls(fullAfter = 9)
        assertEquals(RemotePenIntake.Result.Refused, intake.admit((1..12).toList(), full.reserve, full.enqueue))
        assertEquals(listOf("reserve:8", "reserve:4") + (1..10).map { "enqueue:$it" }, full.log)
    }

    private val ms = 1_000_000L
    /** One frame of a 120 Hz display. */
    private val frame = 8_333_333L

    @Test fun pacer_lets_eight_go_within_one_frame_as_today_and_never_delays_ordinary_drawing() {
        val pacer = RemoteSendPacer(windowNanos = frame)
        var now = 5_000 * ms
        repeat(8) { assertEquals(0L, pacer.acquire(now)) }
        // A ninth must wait until the first of the eight is a whole frame old.
        assertEquals(frame, pacer.acquire(now))
        now += frame / 4
        assertEquals(frame - frame / 4, pacer.acquire(now))
        now += frame - frame / 4 - 1
        assertEquals(1L, pacer.acquire(now))
        // One frame after the eight, the next eight may go: two events a frame apart, as today.
        now += 1
        repeat(8) { assertEquals(0L, pacer.acquire(now)) }
        assertEquals(frame, pacer.acquire(now))
        // A long pause never allows more than eight at once.
        now += 10_000 * ms
        repeat(8) { assertEquals(0L, pacer.acquire(now)) }
        assertEquals(frame, pacer.acquire(now))
        // Four samples per frame (a 480 Hz pen) for a second, delivered per frame or one by one: never a wait.
        val batched = RemoteSendPacer(windowNanos = frame)
        var vsync = 0L
        repeat(120) { repeat(4) { assertEquals(0L, batched.acquire(vsync)) }; vsync += frame }
        val unbuffered = RemoteSendPacer(windowNanos = frame)
        for (sample in 0 until 480) assertEquals(0L, unbuffered.acquire(sample * (frame / 4)))
        // A clock that steps backwards cannot mint slots or wait longer than one frame.
        val backwards = RemoteSendPacer(windowNanos = frame)
        repeat(8) { assertEquals(0L, backwards.acquire(1_000 * ms)) }
        assertEquals(frame, backwards.acquire(900 * ms))
        assertEquals(frame - ms, backwards.acquire(901 * ms))
        assertEquals(0L, backwards.acquire(900 * ms + frame))
    }

    @Test fun in_no_frame_long_window_does_the_computer_get_more_than_the_eight_it_can_get_today() {
        // Whatever arrives, however bunched: release times never put nine samples within one frame.
        for (burst in listOf(9, 12, 16, 24, 32, 100)) {
            val pacer = RemoteSendPacer(windowNanos = frame)
            var now = 0L
            val sentAt = mutableListOf<Long>()
            repeat(burst) {
                while (true) { val wait = pacer.acquire(now); if (wait <= 0L) break; now += wait }
                sentAt += now
                now += 100_000 // The sender itself takes a moment per sample.
            }
            for (start in sentAt) assertTrue("burst=$burst", sentAt.count { it >= start && it < start + frame } <= 8)
            // Sixteen go as eight and, a frame later, eight more.
            if (burst == 16) { assertTrue(sentAt[7] < ms); assertEquals(frame, sentAt[8]); assertTrue(sentAt[15] < frame + ms) }
        }
    }

    @Test fun sends_are_paced_only_with_a_switch_that_can_hand_over_more_than_one_frames_samples() {
        // Both off: no pacer exists, so sending is exactly as before.
        assertFalse(remoteSendPaced(longBatches = false, unbufferedPen = false))
        assertTrue(remoteSendPaced(longBatches = true, unbufferedPen = false))
        assertTrue(remoteSendPaced(longBatches = false, unbufferedPen = true))
        assertTrue(remoteSendPaced(longBatches = true, unbufferedPen = true))
    }

    @Test fun the_pacer_follows_the_displays_own_frame_time() {
        assertEquals(8_333_333L, remoteFrameNanos(120f)); assertEquals(16_666_666L, remoteFrameNanos(60f)); assertEquals(11_111_111L, remoteFrameNanos(90f))
        // Unknown or implausible: the slow frame, which only ever paces more.
        for (rate in listOf(0f, -60f, 10f, 1_000f, Float.NaN, Float.POSITIVE_INFINITY)) assertEquals(RemoteSendPacer.SLOW_FRAME_NANOS, remoteFrameNanos(rate))
        val pacer = RemoteSendPacer()
        assertEquals(RemoteSendPacer.SLOW_FRAME_NANOS, pacer.windowNanos)
        repeat(8) { assertEquals(0L, pacer.acquire(0L)) }
        assertEquals(RemoteSendPacer.SLOW_FRAME_NANOS, pacer.acquire(0L))
        pacer.windowNanos = remoteFrameNanos(120f)
        assertEquals(8_333_333L, pacer.acquire(0L)); assertEquals(0L, pacer.acquire(8_333_333L))
    }

    @Test fun a_slot_that_was_not_used_is_given_back_and_never_adds_a_ninth() {
        // Looking for a second sample that is not there costs nothing: a pen
        // reporting every millisecond for a second is paced exactly as if nobody had looked.
        val looked = RemoteSendPacer(windowNanos = frame); val plain = RemoteSendPacer(windowNanos = frame)
        var now = 0L
        repeat(1_000) {
            val wait = plain.acquire(now)
            assertEquals(wait, looked.acquire(now))
            if (wait == 0L && looked.acquire(now) == 0L) looked.giveBack() // A look for one more; none was waiting.
            now += ms
        }
        // A slot given back is the very next one granted, without waiting.
        val full = RemoteSendPacer(windowNanos = frame)
        repeat(8) { assertEquals(0L, full.acquire(0L)) }
        assertEquals(frame, full.acquire(0L))
        full.giveBack(); full.giveBack() // Only the one slot that was granted last can come back.
        assertEquals(0L, full.acquire(0L))
        assertEquals(frame, full.acquire(0L))
    }

    @Test fun typed_batches_send_together_what_is_already_waiting_up_to_eight() {
        fun waiting(count: Int) = ArrayDeque((2..count).toList())
        // No pacer: the first sample and up to seven more that are waiting.
        val three = waiting(3)
        assertEquals(listOf(1, 2, 3), remotePenGroup(1, null, { 0L }) { three.removeFirstOrNull() })
        val twenty = waiting(20)
        assertEquals((1..8).toList(), remotePenGroup(1, null, { 0L }) { twenty.removeFirstOrNull() })
        assertEquals(12, twenty.size)
        assertEquals(listOf(1), remotePenGroup(1, null, { 0L }) { null })
        // Paced: the first sample already holds a slot; each further one needs its own.
        val pacer = RemoteSendPacer(windowNanos = frame)
        assertEquals(0L, pacer.acquire(0L))
        val paced = waiting(20)
        assertEquals((1..8).toList(), remotePenGroup(1, pacer, { 0L }) { paced.removeFirstOrNull() })
        // The frame's eight slots are used: the next call takes one sample and no more.
        assertEquals(frame, pacer.acquire(0L))
        assertEquals(0L, pacer.acquire(frame))
        assertEquals((9..16).toList(), remotePenGroup(paced.removeFirst(), pacer, { frame }) { paced.removeFirstOrNull() })
        // Nothing waiting: the slot taken for the look is given back, so the next sample still goes at once.
        val idle = RemoteSendPacer(windowNanos = frame)
        repeat(7) { assertEquals(0L, idle.acquire(0L)) }
        assertEquals(listOf(7), remotePenGroup(7, idle, { 0L }) { null })
        assertEquals(0L, idle.acquire(0L)); assertEquals(frame, idle.acquire(0L))
    }
}
