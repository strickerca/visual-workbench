package com.visualworkbench.android.diagnostics

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
    RemoteSent("remote_sent", "Remote: pen sample to sent"),
    RemoteEcho("remote_echo", "Remote: pen sample to frame that shows it"),
}

/** Durations in microseconds; percentiles are nearest-rank over the window. */
internal data class TimingSummary(val count: Int, val total: Long, val p50Micros: Long, val p95Micros: Long, val maxMicros: Long) {
    companion object { val EMPTY = TimingSummary(0, 0L, 0L, 0L, 0L) }
}

/** The newest [capacity] durations of one timing. Any thread may record. */
internal class TimingWindow(private val capacity: Int = 1024) {
    private val values = LongArray(capacity)
    private var next = 0
    private var size = 0
    private var total = 0L
    init { require(capacity in 1..65_536) }

    /** Negative or absurd durations (a clock that jumped) are dropped, not clamped. */
    @Synchronized fun record(micros: Long) {
        if (micros < 0 || micros > MAX_MICROS) return
        values[next] = micros; next = (next + 1) % capacity
        if (size < capacity) size++
        if (total < Long.MAX_VALUE) total++
    }
    @Synchronized fun clear() { next = 0; size = 0; total = 0L }
    @Synchronized fun summary(): TimingSummary {
        if (size == 0) return TimingSummary.EMPTY
        val ordered = values.copyOf(size).also { it.sort() }
        fun rank(percent: Int): Long = ordered[((size * percent + 99) / 100 - 1).coerceIn(0, size - 1)]
        return TimingSummary(size, total, rank(50), rank(95), ordered[size - 1])
    }
    private companion object { const val MAX_MICROS = 600_000_000L }
}

/** Process-local pen-path measurements for the diagnostics screen. Recording is
 * a few array writes; it never allocates per sample and never blocks drawing. */
internal class PenPathMetrics {
    private val windows = PenTiming.entries.associateWith { TimingWindow() }
    private var lastFirstInk = Long.MIN_VALUE

    fun record(timing: PenTiming, micros: Long) { windows.getValue(timing).record(micros) }
    fun recordMillis(timing: PenTiming, millis: Long) { if (millis in 0..600_000) record(timing, millis * 1_000L) }
    fun recordNanos(timing: PenTiming, nanos: Long) { if (nanos >= 0) record(timing, nanos / 1_000L) }

    /** One ink frame was drawn. [newestSampleMs] is the newest real pen sample
     * in that frame and [strokeStartMs] the pen-down of its newest stroke, both
     * in the input event clock; [drawnMs] is the same clock when drawing ended.
     * The first frame of each stroke also counts as that stroke's first ink. */
    fun inkDrawn(newestSampleMs: Long, strokeStartMs: Long, drawnMs: Long, drawNanos: Long) {
        if (newestSampleMs <= 0) return
        recordMillis(PenTiming.CanvasInkDrawn, drawnMs - newestSampleMs)
        recordNanos(PenTiming.CanvasDrawTime, drawNanos)
        val first = synchronized(this) { if (strokeStartMs > 0 && strokeStartMs != lastFirstInk) { lastFirstInk = strokeStartMs; true } else false }
        if (first) recordMillis(PenTiming.CanvasFirstInk, drawnMs - strokeStartMs)
    }
    fun frameReady(newestSampleMs: Long, readyMs: Long) { if (newestSampleMs > 0) recordMillis(PenTiming.CanvasFrameReady, readyMs - newestSampleMs) }

    fun summary(timing: PenTiming): TimingSummary = windows.getValue(timing).summary()
    fun clear() { windows.values.forEach { it.clear() }; synchronized(this) { lastFirstInk = Long.MIN_VALUE } }

    /** Plain text, one line per timing that has data, then the switched-on
     * developer switches by stable key. This exact text is what "Copy" places
     * on the clipboard and what `trace_tool.py timings` reads. */
    fun report(switchesOn: List<String>): String = buildString {
        append("pen_path_timings v1\n")
        for (timing in PenTiming.entries) {
            val s = summary(timing)
            if (s.count == 0) continue
            append(timing.key).append(" n=").append(s.count).append(" p50_ms=").append(ms(s.p50Micros))
                .append(" p95_ms=").append(ms(s.p95Micros)).append(" max_ms=").append(ms(s.maxMicros)).append('\n')
        }
        append("switches=").append(switchesOn.sorted().joinToString(",").ifEmpty { "none" }).append('\n')
    }

    companion object {
        /** Milliseconds with one decimal, locale independent. */
        fun ms(micros: Long): String { val tenths = (micros + 50) / 100; return "${tenths / 10}.${tenths % 10}" }
    }
}
