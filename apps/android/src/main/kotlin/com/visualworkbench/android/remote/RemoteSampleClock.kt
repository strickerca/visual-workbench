package com.visualworkbench.android.remote

/** Places a pen sample's own event time on this phone's System.nanoTime clock.
 *
 * Android stamps each sample when the digitizer reports it, in the uptime
 * clock. Ghost ink, the 50 ms staleness rule and the send-to-echo timing all
 * work in System.nanoTime. The original conversion did not assume the two are
 * the same clock: it aged every sample against the next whole uptime
 * millisecond, read when the event was handled. That is safe but makes each
 * sample up to 1 ms older than it is, by an amount that depends on the moment
 * of handling, so two samples from different events are spaced by their
 * arrival as well as by the pen.
 *
 * With [exact], when both clocks are seen to agree at the moment of handling,
 * the sample keeps exactly its own event time. Otherwise, and always without
 * [exact], the original conversion is used unchanged. */
internal class RemoteSampleClock(val exact: Boolean) {
    /** The sample's age in nanoseconds at the moment of handling; its time on
     * the System.nanoTime clock is [nowNanos] minus this. [eventNanos] is the
     * sample's uptime-clock event time. [nowNanos] is System.nanoTime() and
     * [uptimeMillis] SystemClock.uptimeMillis(), read in that order when the
     * event is handled. A negative or too large age is the caller's refusal,
     * exactly as before. A plain number: nothing is allocated per sample. */
    fun ageNanos(eventNanos: Long, nowNanos: Long, uptimeMillis: Long): Long =
        if (exact && sameClock(nowNanos, uptimeMillis)) nowNanos - eventNanos
        // Align without assuming a shared clock. Uptime's millisecond upper
        // edge conservatively adds at most 1 ms of age.
        else (uptimeMillis + 1L) * 1_000_000L - eventNanos

    companion object {
        /** [nowNanos] was read just before [uptimeMillis]. On one shared clock
         * it lies below the end of that millisecond and, allowing a whole
         * millisecond for being descheduled between the two reads, not before
         * the start of the previous one. Two unrelated clocks do not land in
         * that window. */
        fun sameClock(nowNanos: Long, uptimeMillis: Long): Boolean =
            uptimeMillis in 1L..9_000_000_000_000L &&
                nowNanos < (uptimeMillis + 1L) * 1_000_000L && nowNanos >= (uptimeMillis - 1L) * 1_000_000L
    }
}
