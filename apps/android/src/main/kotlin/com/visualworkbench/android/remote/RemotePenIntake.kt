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
    val reservation: Int get() = if (longBatches && typed) TYPED_SAMPLES else GROUP

    /** [reserve] is the short native state-only reservation: false means an
     * editor command is refreshing and no sample of this event may be queued.
     * [enqueue] is false when the phone queue is full. Call under the input
     * fence. After [Result.Refused] the caller clears the queue and pauses,
     * which also retires every reservation made here. Inline: the two calls
     * are made in place, as they were written in the controller before, and
     * nothing is allocated per event. */
    inline fun <T> admit(batch: List<T>, reserve: (UInt) -> Boolean, enqueue: (T) -> Boolean): Result {
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

/** Whether sends to the computer are paced. Without either switch one event
 * carries at most eight samples and events are a display frame apart, so the
 * computer's eight-slot queue cannot be overrun and there is no pacer, as
 * before. Long batches hand the sender more than eight samples of one event;
 * unbuffered delivery can hand it several small events within a millisecond
 * after the main thread was held up. Either one switches the pacer on. */
internal fun remoteSendPaced(longBatches: Boolean, unbufferedPen: Boolean): Boolean = longBatches || unbufferedPen

/** One display frame in nanoseconds for a display that reports [refreshRate];
 * a slow 60 Hz frame when the rate is missing or implausible. */
internal fun remoteFrameNanos(refreshRate: Float): Long =
    if (refreshRate.isFinite() && refreshRate >= 24f && refreshRate <= 240f) (1_000_000_000.0 / refreshRate).toLong()
    else RemoteSendPacer.SLOW_FRAME_NANOS

/** Typed batches: [first] and whatever else is already waiting, up to eight
 * samples for one call. With paced sends each further sample needs its own
 * send slot; a slot taken for a sample that is not there is given back. */
internal inline fun <T> remotePenGroup(first: T, pacer: RemoteSendPacer?, now: () -> Long, next: () -> T?): List<T> {
    val group = ArrayList<T>(RemotePenIntake.GROUP)
    group.add(first)
    while (group.size < RemotePenIntake.GROUP) {
        if (pacer != null && pacer.acquire(now()) > 0L) break
        val more = next()
        if (more == null) { pacer?.giveBack(); break }
        group.add(more)
    }
    return group
}

/** Releases queued samples to the computer no faster than it can get them
 * today. Without the switches one pen event carries at most eight samples and
 * events are a display frame apart, so the computer is never handed more than
 * eight samples within one frame time. This lets a send go only while fewer
 * than eight were sent within the last frame time, which is that same bound
 * for whatever the switches queue: a longer event goes as eight at once and
 * the rest a frame later, exactly as two events do today. In ordinary drawing
 * (a pen sample every two to four milliseconds) it never waits. Used only
 * when [remoteSendPaced]; one sender owns it, and the view reports the
 * display's frame time through [windowNanos]. */
internal class RemoteSendPacer(private val burst: Int = RemotePenIntake.GROUP, windowNanos: Long = SLOW_FRAME_NANOS) {
    /** One display frame. Until the view has reported its display, a slow 60 Hz frame. */
    @Volatile var windowNanos: Long = windowNanos
    /** When each of the last [burst] samples was released, oldest at [next]. */
    private val sent = LongArray(burst) { NEVER }
    private var next = 0
    private var replaced = NEVER
    private var returnable = false

    /** Takes one send slot and returns 0, or returns the nanoseconds to wait
     * before asking again. [nowNanos] is a monotonic clock. */
    fun acquire(nowNanos: Long): Long {
        val oldest = sent[next]
        if (oldest != NEVER) {
            // A clock that steps backwards mints no slot and costs one frame, not the size of the step.
            if (nowNanos < oldest) { sent.fill(nowNanos); returnable = false; return windowNanos }
            val wait = windowNanos - (nowNanos - oldest)
            if (wait > 0L) return wait
        }
        replaced = oldest; sent[next] = nowNanos; next = (next + 1) % burst; returnable = true
        return 0L
    }

    /** Returns the slot granted last, which was not used for a send. Once per granted slot. */
    fun giveBack() {
        if (!returnable) return
        returnable = false; next = (next + burst - 1) % burst; sent[next] = replaced
    }

    companion object {
        private const val NEVER = Long.MIN_VALUE
        const val SLOW_FRAME_NANOS = 16_666_667L
    }
}
