package com.visualworkbench.android.remote

import android.view.MotionEvent
import org.junit.Assert.*
import org.junit.Test

class RemoteSampleClockTest {
    private val ms = 1_000_000L

    private data class Sample(val localNanos: Long, val ageNanos: Long)
    /** The numbers the controller computed inline before the switch existed. */
    private fun original(eventNanos: Long, nowNanos: Long, uptimeMillis: Long): Sample {
        val age = (uptimeMillis + 1L) * 1_000_000L - eventNanos
        return Sample(nowNanos - age, age)
    }
    /** As the controller uses it: the age, and the sample's time as now minus that age. */
    private fun RemoteSampleClock.sample(eventNanos: Long, nowNanos: Long, uptimeMillis: Long): Sample {
        val age = ageNanos(eventNanos, nowNanos, uptimeMillis)
        return Sample(nowNanos - age, age)
    }

    @Test fun switched_off_gives_the_same_time_and_age_as_before_for_every_case() {
        val clock = RemoteSampleClock(exact = false)
        val uptimes = listOf(1L, 5_000L, 86_400_000L)
        val phases = listOf(0L, 1L, 499_999L, 999_999L)
        val ages = listOf(-2 * ms, 0L, 1L, 4_166_667L, 49 * ms, 50 * ms, 400 * ms)
        // Shared clock, an unrelated clock ahead and one behind: all handled alike.
        for (offset in listOf(0L, 7_919 * ms + 13, -3_301 * ms - 7)) for (uptime in uptimes) for (phase in phases) for (age in ages) {
            val uptimeNanos = uptime * ms + phase
            val now = uptimeNanos + offset
            val event = uptimeNanos - age
            assertEquals(original(event, now, uptime), clock.sample(event, now, uptime))
        }
    }

    @Test fun switched_on_keeps_each_samples_own_time_when_the_clocks_agree() {
        val clock = RemoteSampleClock(exact = true)
        // Two samples 4.166 ms apart, handled in different events at different
        // sub-millisecond moments: their spacing is the pen's, not the handling's.
        val first = 10_000 * ms + 123_456
        val second = first + 4_166_667
        val a = clock.sample(first, first + 2 * ms + 950_000, (first + 2 * ms + 950_000) / ms)
        val b = clock.sample(second, second + 1 * ms + 20_000, (second + 1 * ms + 20_000) / ms)
        assertEquals(first, a.localNanos); assertEquals(second, b.localNanos)
        assertEquals(4_166_667L, b.localNanos - a.localNanos)
        assertEquals(2 * ms + 950_000, a.ageNanos); assertEquals(1 * ms + 20_000, b.ageNanos)
        // The original conversion spaces the same two samples by their handling too.
        val legacy = RemoteSampleClock(exact = false)
        val la = legacy.sample(first, first + 2 * ms + 950_000, (first + 2 * ms + 950_000) / ms)
        val lb = legacy.sample(second, second + 1 * ms + 20_000, (second + 1 * ms + 20_000) / ms)
        assertNotEquals(4_166_667L, lb.localNanos - la.localNanos)
        // Never later than the sample really was, so the staleness rule stays on the safe side.
        assertTrue(la.localNanos <= first && lb.localNanos <= second)
        assertTrue(first - la.localNanos <= ms && second - lb.localNanos <= ms)
    }

    @Test fun switched_on_falls_back_to_the_original_conversion_when_the_clocks_do_not_agree() {
        val clock = RemoteSampleClock(exact = true)
        for (offset in listOf(3 * ms, -3 * ms, 86_400_000 * ms)) {
            val uptime = 20_000L
            val now = uptime * ms + 400_000 + offset
            val event = uptime * ms - 5 * ms
            assertFalse(RemoteSampleClock.sameClock(now, uptime))
            assertEquals(original(event, now, uptime), clock.sample(event, now, uptime))
        }
        // Read order: nanoTime first, then uptime. A thread descheduled for most
        // of a millisecond between the two reads is still recognised.
        assertTrue(RemoteSampleClock.sameClock(20_000 * ms + 1, 20_000))
        assertTrue(RemoteSampleClock.sameClock(20_000 * ms - 900_000, 20_000))
        assertTrue(RemoteSampleClock.sameClock(20_000 * ms + 999_999, 20_000))
        assertFalse(RemoteSampleClock.sameClock(20_001 * ms, 20_000))
        assertFalse(RemoteSampleClock.sameClock(19_999 * ms - 1, 20_000))
        assertFalse(RemoteSampleClock.sameClock(0, 0))
        // A sample from the future or older than 50 ms still shows as such to the caller.
        val future = clock.sample(30_000 * ms + 600_000, 30_000 * ms + 500_000, 30_000)
        assertTrue(future.ageNanos < 0)
        assertEquals(60 * ms, clock.sample(30_000 * ms - 60 * ms, 30_000 * ms, 30_000).ageNanos)
    }

    @Test fun unbuffered_delivery_is_requested_only_at_a_pen_down_and_only_when_switched_on() {
        val actions = listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL,
            MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_EXIT)
        val tools = listOf(MotionEvent.TOOL_TYPE_STYLUS, MotionEvent.TOOL_TYPE_ERASER, MotionEvent.TOOL_TYPE_FINGER,
            MotionEvent.TOOL_TYPE_MOUSE, MotionEvent.TOOL_TYPE_UNKNOWN)
        for (action in actions) for (tool in tools) {
            assertFalse(remoteUnbufferedPenRequest(false, action, tool))
            val pen = tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER
            assertEquals(action == MotionEvent.ACTION_DOWN && pen, remoteUnbufferedPenRequest(true, action, tool))
        }
    }
}
