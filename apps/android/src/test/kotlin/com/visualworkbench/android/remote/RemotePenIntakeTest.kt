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

    @Test fun pacer_releases_eight_at_once_then_one_per_millisecond_and_never_delays_ordinary_drawing() {
        val ms = 1_000_000L
        val pacer = RemoteSendPacer()
        var now = 5_000 * ms
        repeat(8) { assertEquals(0L, pacer.acquire(now)) }
        assertEquals(ms, pacer.acquire(now))
        now += ms / 4
        assertEquals(ms - ms / 4, pacer.acquire(now))
        now += ms - ms / 4
        assertEquals(0L, pacer.acquire(now)); assertEquals(ms, pacer.acquire(now))
        // Three milliseconds later exactly three more may go.
        now += 3 * ms
        repeat(3) { assertEquals(0L, pacer.acquire(now)) }
        assertTrue(pacer.acquire(now) in 1..ms)
        // A long pause refills to eight, never more.
        now += 10_000 * ms
        repeat(8) { assertEquals(0L, pacer.acquire(now)) }
        assertEquals(ms, pacer.acquire(now))
        // Four samples per 120 Hz frame (a 480 Hz pen) for a second: never a wait.
        val steady = RemoteSendPacer()
        var frame = 0L
        repeat(120) { repeat(4) { assertEquals(0L, steady.acquire(frame)) }; frame += 8_333_333L }
        // A clock that steps backwards cannot mint slots or wait forever.
        val backwards = RemoteSendPacer()
        repeat(8) { assertEquals(0L, backwards.acquire(1_000 * ms)) }
        assertEquals(ms, backwards.acquire(900 * ms))
        assertEquals(0L, backwards.acquire(901 * ms))
    }

    @Test fun sends_are_paced_only_with_a_switch_that_can_hand_over_more_than_one_frames_samples() {
        // Both off: no pacer exists, so sending is exactly as before.
        assertFalse(remoteSendPaced(longBatches = false, unbufferedPen = false))
        assertTrue(remoteSendPaced(longBatches = true, unbufferedPen = false))
        assertTrue(remoteSendPaced(longBatches = false, unbufferedPen = true))
        assertTrue(remoteSendPaced(longBatches = true, unbufferedPen = true))
    }

    @Test fun a_slot_that_was_not_used_is_given_back_and_never_adds_a_ninth() {
        val ms = 1_000_000L
        val pacer = RemoteSendPacer()
        // A full bucket stays at eight.
        pacer.giveBack()
        repeat(8) { assertEquals(0L, pacer.acquire(0L)) }
        assertEquals(ms, pacer.acquire(0L))
        // Looking for a second sample that is not there costs nothing: a pen
        // reporting every millisecond for a second never waits.
        val steady = RemoteSendPacer()
        var now = 0L
        repeat(1_000) {
            assertEquals(0L, steady.acquire(now))   // the sample itself
            assertEquals(0L, steady.acquire(now))   // a look for one more
            steady.giveBack()                       // none was waiting
            now += ms
        }
        // A slot given back is the very next one granted, without waiting.
        val empty = RemoteSendPacer()
        repeat(8) { assertEquals(0L, empty.acquire(0L)) }
        assertEquals(ms, empty.acquire(0L))
        empty.giveBack()
        assertEquals(0L, empty.acquire(0L))
        assertEquals(ms, empty.acquire(0L))
    }

    @Test fun a_sixteen_sample_burst_reaches_the_computer_as_eight_then_eight_within_about_eight_milliseconds() {
        val ms = 1_000_000L
        val pacer = RemoteSendPacer()
        var now = 0L
        val sentAt = mutableListOf<Long>()
        repeat(16) {
            while (true) { val wait = pacer.acquire(now); if (wait <= 0L) break; now += wait }
            sentAt += now
        }
        assertEquals(List(8) { 0L }, sentAt.take(8))
        assertEquals((1..8).map { it * ms }, sentAt.drop(8))
        // In any eight-millisecond window at most sixteen samples leave: no more
        // than two back-to-back eight-sample events can produce today.
        for (start in sentAt) assertTrue(sentAt.count { it >= start && it < start + 8 * ms } <= 16)
    }
}
