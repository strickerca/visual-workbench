package com.visualworkbench.android.diagnostics

import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicLongArray

/** The fixed set of pen-path timings. Only these static names, durations and
 * counts are ever kept: no coordinates, pressure, object or device identity.
 * [key] is the stable name used in the copied text and by trace_tool.py. */
internal enum class PenTiming(val key: String, val label: String) {
    CanvasInkDrawn("canvas_ink_drawn", "Canvas: newest pen sample to ink drawn"),
    CanvasFrameReady("canvas_frame_ready", "Canvas: newest pen sample to frame ready"),
    CanvasDrawTime("canvas_draw_time", "Canvas: time to draw one ink frame"),
    CanvasFirstInk("canvas_first_ink", "Canvas: pen-down to first ink"),
    CanvasStrokeStart("canvas_stroke_start", "Canvas: pen-down to stroke accepted"),
    CanvasSave("canvas_save", "Canvas: pen-up to saved"),
    CanvasRefresh("canvas_refresh", "Canvas: saved to canvas refreshed"),
    CanvasSavedDraw("canvas_saved_draw", "Canvas: time to draw one saved frame"),
    CanvasSavedReady("canvas_saved_ready", "Canvas: saved frame drawn to frame ready"),
    RemoteSent("remote_sent", "Remote: pen sample to sent"),
    RemoteEcho("remote_echo", "Remote: pen sample to frame that shows it"),
}

/** Why a pen-down was not acted on. [key] is the stable name in the copied
 * text; [label] is shown to the owner. Saving or showing earlier strokes is
 * not among them: [Backlog] is the bound on ink still unsaved, which drawing
 * does not reach, and [Lists] never applies to ink. */
internal enum class PenRefusal(val key: String, val label: String) {
    Backlog("backlog", "too much ink still unsaved"),
    Busy("busy", "a project was being opened, closed or transferred"),
    Modal("modal", "a dialog, panel or another screen was open"),
    Loading("loading", "the page picture was not shown yet"),
    Stream("stream", "the capture stream panel had the pen"),
    Lists("lists", "the instruction list was being read"),
    Failed("failed", "the stroke could not start"),
}

/** Pen-downs that asked for ink since the counts were last cleared. [refused]
 * lists only the reasons that occurred. */
internal data class PenDownCounts(val accepted: Long, val refused: Map<PenRefusal, Long>) {
    val refusedTotal: Long get() = refused.values.sum()
}

/** Durations in microseconds; percentiles are nearest-rank over the window. */
internal data class TimingSummary(val count: Int, val total: Long, val p50Micros: Long, val p95Micros: Long, val maxMicros: Long) {
    companion object { val EMPTY = TimingSummary(0, 0L, 0L, 0L, 0L) }
}

/** The newest [capacity] durations of one timing. Any thread may record:
 * recording is one counter step and one write into a preallocated array, with
 * no lock and nothing allocated, so it can run on the drawing thread. A
 * summary read while values are being recorded may miss the newest one. */
internal class TimingWindow(private val capacity: Int = 1024) {
    private val values = AtomicLongArray(capacity)
    private val recorded = AtomicLong()
    init { require(capacity in 1..65_536) }

    /** Negative or absurd durations (a clock that jumped) are dropped, not clamped. */
    fun record(micros: Long) {
        if (micros < 0 || micros > MAX_MICROS) return
        values.lazySet((recorded.getAndIncrement() % capacity).toInt(), micros)
    }
    fun clear() { recorded.set(0L) }
    fun summary(): TimingSummary {
        val total = recorded.get()
        val size = minOf(total, capacity.toLong()).toInt()
        if (size <= 0) return TimingSummary.EMPTY
        val ordered = LongArray(size) { values.get(it) }.also { it.sort() }
        fun rank(percent: Int): Long = ordered[((size * percent + 99) / 100 - 1).coerceIn(0, size - 1)]
        return TimingSummary(size, total, rank(50), rank(95), ordered[size - 1])
    }
    private companion object { const val MAX_MICROS = 600_000_000L }
}

/** Process-local pen-path measurements for the diagnostics screen. Recording
 * takes no lock and allocates nothing; it never blocks drawing. */
internal class PenPathMetrics {
    private val windows = Array(PenTiming.entries.size) { TimingWindow() }
    private val lastInkSample = AtomicLong(NONE)
    private val lastReadySample = AtomicLong(NONE)
    private val lastFirstInk = AtomicLong(NONE)
    private val penDownsAccepted = AtomicLong()
    private val penDownsRefused = AtomicLongArray(PenRefusal.entries.size)

    fun record(timing: PenTiming, micros: Long) { windows[timing.ordinal].record(micros) }
    fun recordMillis(timing: PenTiming, millis: Long) { if (millis in 0..600_000) record(timing, millis * 1_000L) }
    fun recordNanos(timing: PenTiming, nanos: Long) { if (nanos >= 0) record(timing, nanos / 1_000L) }

    /** One ink frame was drawn. [newestSampleMs] is the newest real pen sample
     * in that frame and [strokeStartMs] the pen-down of its stroke, both in the
     * input event clock; [drawnMs] is the same clock when drawing ended.
     *
     * Only a frame that shows a pen sample for the first time is a pen-to-ink
     * measurement. The same ink is drawn again for a hover, a status change,
     * after a saved frame, and for as long as a finished stroke waits to be
     * saved; timing those against the old sample would report the length of
     * the save as ink latency. The first counted frame of each stroke is also
     * that stroke's first ink. */
    fun inkDrawn(newestSampleMs: Long, strokeStartMs: Long, drawnMs: Long, drawNanos: Long) {
        if (newestSampleMs <= 0 || lastInkSample.getAndSet(newestSampleMs) == newestSampleMs) return
        recordMillis(PenTiming.CanvasInkDrawn, drawnMs - newestSampleMs)
        recordNanos(PenTiming.CanvasDrawTime, drawNanos)
        if (strokeStartMs > 0 && lastFirstInk.getAndSet(strokeStartMs) != strokeStartMs) recordMillis(PenTiming.CanvasFirstInk, drawnMs - strokeStartMs)
    }
    /** The frame with [newestSampleMs] was handed to the display; counted once per sample, as [inkDrawn]. */
    fun frameReady(newestSampleMs: Long, readyMs: Long) {
        if (newestSampleMs <= 0 || lastReadySample.getAndSet(newestSampleMs) == newestSampleMs) return
        recordMillis(PenTiming.CanvasFrameReady, readyMs - newestSampleMs)
    }

    /** One pen-down that asked for ink: taken ([refusal] null), or not and why. */
    fun penDown(refusal: PenRefusal?) {
        if (refusal == null) penDownsAccepted.incrementAndGet() else penDownsRefused.incrementAndGet(refusal.ordinal)
    }
    fun penDowns(): PenDownCounts = PenDownCounts(penDownsAccepted.get(),
        PenRefusal.entries.associateWith { penDownsRefused.get(it.ordinal) }.filterValues { it != 0L })

    fun summary(timing: PenTiming): TimingSummary = windows[timing.ordinal].summary()
    fun clear() {
        windows.forEach { it.clear() }
        lastInkSample.set(NONE); lastReadySample.set(NONE); lastFirstInk.set(NONE)
        penDownsAccepted.set(0L); for (reason in PenRefusal.entries) penDownsRefused.set(reason.ordinal, 0L)
    }

    /** Plain text, one line per timing that has data, then the pen-downs that
     * asked for ink (once there were any: how many were taken, how many were
     * refused, and each reason that occurred), then the switched-on developer
     * switches by stable key. This exact text is what "Copy" places on the
     * clipboard and what `trace_tool.py timings` reads. */
    fun report(switchesOn: List<String>): String = buildString {
        append("pen_path_timings v1\n")
        for (timing in PenTiming.entries) {
            val s = summary(timing)
            if (s.count == 0) continue
            append(timing.key).append(" n=").append(s.count).append(" p50_ms=").append(ms(s.p50Micros))
                .append(" p95_ms=").append(ms(s.p95Micros)).append(" max_ms=").append(ms(s.maxMicros)).append('\n')
        }
        val downs = penDowns()
        if (downs.accepted != 0L || downs.refusedTotal != 0L) {
            append("pen_downs accepted=").append(downs.accepted).append(" refused=").append(downs.refusedTotal)
            for ((reason, count) in downs.refused) append(' ').append(reason.key).append('=').append(count)
            append('\n')
        }
        append("switches=").append(switchesOn.sorted().joinToString(",").ifEmpty { "none" }).append('\n')
    }

    companion object {
        private const val NONE = Long.MIN_VALUE
        /** One line for the diagnostics screen: how many pen-downs that asked
         * for ink were drawn, and how many were not and why. */
        fun penDownsLine(counts: PenDownCounts): String = "Pen-downs for ink: ${counts.accepted} drawn, " +
            if (counts.refusedTotal == 0L) "none refused"
            else "${counts.refusedTotal} refused (" + counts.refused.entries.joinToString("; ") { (reason, count) -> "${reason.label}: $count" } + ")"
        /** Milliseconds with one decimal, locale independent. */
        fun ms(micros: Long): String { val tenths = (micros + 50) / 100; return "${tenths / 10}.${tenths % 10}" }
    }
}
