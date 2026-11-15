package com.visualworkbench.android.remote

/** How one pen event's samples enter the bounded phone queue.
 *
 * The computer takes pen samples through an eight-slot queue and ends the
 * connection when it overflows, so the phone never hands it more than eight at
 * once. Until now an event with more than eight samples (the main thread was
 * held up for a few frames) was refused and input paused. With [longBatches]
 * such an event is reserved in groups of at most eight and its samples are
 * later released to the computer by [RemoteSendPacer]. Without it this class
 * makes exactly the calls the controller made before: one reservation for the
 * whole event, then one enqueue per sample, stopping at the first refusal. */
internal class RemotePenIntake(val longBatches: Boolean, val typed: Boolean = false) {
    enum class Result { Admitted, CommandBusy, Refused }

    /** Largest MotionEvent history accepted; one more sample is the event itself. */
    val maxHistory: Int get() = maxSamples - 1
    /** Samples of one event: 8 as before; 16 with long batches (two reservations
     * of the original call, whose queue holds 16); 32 when the typed reservation,
     * which has room for 64, is used as well. */
    val maxSamples: Int get() = if (!longBatches) GROUP else if (typed) TYPED_SAMPLES else LONG_SAMPLES
    /** Samples that may wait on the phone between the touch thread and the sender. */
    val queueCapacity: Int get() = if (!longBatches) GROUP else if (typed) TYPED_QUEUE else LONG_QUEUE
    /** Largest single reservation: 8 for the original call, a whole event for the typed one. */
    private val reservation: Int get() = if (longBatches && typed) TYPED_SAMPLES else GROUP

    /** [reserve] is the short native state-only reservation: false means an
     * editor command is refreshing and no sample of this event may be queued.
     * [enqueue] is false when the phone queue is full. Call under the input
     * fence. After [Result.Refused] the caller clears the queue and pauses,
     * which also retires every reservation made here. */
    fun <T> admit(batch: List<T>, reserve: (UInt) -> Boolean, enqueue: (T) -> Boolean): Result {
        if (!longBatches) {
            if (!reserve(batch.size.toUInt())) return Result.CommandBusy
            return if (batch.all(enqueue)) Result.Admitted else Result.Refused
        }
        var reserved = 0
        while (reserved < batch.size) {
            val group = minOf(reservation, batch.size - reserved)
            if (!reserve(group.toUInt())) return if (reserved == 0) Result.CommandBusy else Result.Refused
            reserved += group
        }
        return if (batch.all(enqueue)) Result.Admitted else Result.Refused
    }

    companion object {
        /** The computer's input queue length and the native per-call reservation bound. */
        const val GROUP = 8
        /** Two native reservations: the native queue bound is sixteen samples. */
        const val LONG_SAMPLES = 16
        const val LONG_QUEUE = 32
        /** The typed reservation takes up to 32 samples at once and holds 64. */
        const val TYPED_SAMPLES = 32
        const val TYPED_QUEUE = 64
    }
}

/** Whether the remote surface asks Android for unbatched delivery: only at the
 * pen-down of a stylus or eraser contact, and only while the switch is on. */
internal fun remoteUnbufferedPenRequest(enabled: Boolean, actionMasked: Int, toolType: Int): Boolean =
    enabled && actionMasked == android.view.MotionEvent.ACTION_DOWN &&
        (toolType == android.view.MotionEvent.TOOL_TYPE_STYLUS || toolType == android.view.MotionEvent.TOOL_TYPE_ERASER)

/** Releases queued samples to the computer no faster than its eight-slot input
 * queue is known to take them today: eight at once, then one more per
 * millisecond. In ordinary drawing (two to four samples per frame) it never
 * waits. Used only with long batches; one sender thread owns it. */
internal class RemoteSendPacer(private val burst: Int = RemotePenIntake.GROUP, private val refillNanos: Long = 1_000_000L) {
    private var tokens = burst
    private var updated = Long.MIN_VALUE

    /** Takes one send slot and returns 0, or returns the nanoseconds to wait
     * before asking again. [nowNanos] is a monotonic clock. */
    fun acquire(nowNanos: Long): Long {
        if (updated == Long.MIN_VALUE || nowNanos < updated) updated = nowNanos
        val earned = (nowNanos - updated) / refillNanos
        if (earned > 0) {
            // A full bucket carries no partial progress toward a ninth slot.
            if (tokens + earned >= burst) { tokens = burst; updated = nowNanos }
            else { tokens += earned.toInt(); updated += earned * refillNanos }
        }
        if (tokens > 0) { tokens--; return 0L }
        return refillNanos - (nowNanos - updated)
    }
}
