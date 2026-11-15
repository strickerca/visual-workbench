package com.visualworkbench.android.diagnostics

import org.junit.Assert.*
import org.junit.Test

class PenPathMetricsTest {
    @Test fun percentiles_are_nearest_rank_over_the_newest_window() {
        val window = TimingWindow(capacity = 100)
        assertEquals(TimingSummary.EMPTY, window.summary())
        for (value in 1L..100L) window.record(value * 1_000)
        val all = window.summary()
        assertEquals(100, all.count); assertEquals(100L, all.total)
        assertEquals(50_000L, all.p50Micros); assertEquals(95_000L, all.p95Micros); assertEquals(100_000L, all.maxMicros)
        // Twenty newer values push the oldest twenty out of the window.
        for (value in 101L..120L) window.record(value * 1_000)
        val newest = window.summary()
        assertEquals(100, newest.count); assertEquals(120L, newest.total)
        assertEquals(70_000L, newest.p50Micros); assertEquals(115_000L, newest.p95Micros); assertEquals(120_000L, newest.maxMicros)
        val one = TimingWindow(4); one.record(7_000)
        assertEquals(TimingSummary(1, 1, 7_000, 7_000, 7_000), one.summary())
        window.clear(); assertEquals(TimingSummary.EMPTY, window.summary())
    }
    @Test fun a_clock_jump_is_dropped_and_never_shown_as_a_timing() {
        val window = TimingWindow(8)
        window.record(-1); window.record(Long.MIN_VALUE); window.record(601_000_000); window.record(Long.MAX_VALUE)
        assertEquals(TimingSummary.EMPTY, window.summary())
        val metrics = PenPathMetrics()
        metrics.recordMillis(PenTiming.CanvasSave, -5); metrics.recordMillis(PenTiming.CanvasSave, 700_000)
        metrics.recordNanos(PenTiming.RemoteSent, -1)
        assertEquals(0, metrics.summary(PenTiming.CanvasSave).count); assertEquals(0, metrics.summary(PenTiming.RemoteSent).count)
    }
    @Test fun an_ink_frame_is_timed_against_its_newest_sample_and_first_ink_once_per_stroke() {
        val metrics = PenPathMetrics()
        // No wet ink in the frame: nothing to time.
        metrics.inkDrawn(newestSampleMs = 0, strokeStartMs = 0, drawnMs = 1_000, drawNanos = 2_000_000)
        assertEquals(0, metrics.summary(PenTiming.CanvasInkDrawn).count)
        // Stroke begun at 1000: frames at 1012 (sample 1004), 1018 (sample 1012), 1024 (sample 1020).
        metrics.inkDrawn(1_004, 1_000, 1_012, 3_000_000)
        metrics.inkDrawn(1_012, 1_000, 1_018, 1_000_000)
        metrics.inkDrawn(1_020, 1_000, 1_024, 2_000_000)
        metrics.frameReady(1_020, 1_027); metrics.frameReady(0, 1_030)
        val drawn = metrics.summary(PenTiming.CanvasInkDrawn)
        assertEquals(3, drawn.count); assertEquals(6_000L, drawn.p50Micros); assertEquals(8_000L, drawn.maxMicros)
        assertEquals(TimingSummary(1, 1, 12_000, 12_000, 12_000), metrics.summary(PenTiming.CanvasFirstInk))
        assertEquals(TimingSummary(1, 1, 7_000, 7_000, 7_000), metrics.summary(PenTiming.CanvasFrameReady))
        assertEquals(2_000L, metrics.summary(PenTiming.CanvasDrawTime).p50Micros)
        // A second stroke gets its own first-ink timing.
        metrics.inkDrawn(2_005, 2_000, 2_009, 1_000_000)
        assertEquals(2, metrics.summary(PenTiming.CanvasFirstInk).count)
        metrics.clear()
        for (timing in PenTiming.entries) assertEquals(0, metrics.summary(timing).count)
        metrics.inkDrawn(2_013, 2_000, 2_016, 1_000_000)
        assertEquals(1, metrics.summary(PenTiming.CanvasFirstInk).count)
    }
    /** Privacy rule: the copied text holds static names, durations and counts only. */
    @Test fun the_copied_text_contains_only_fixed_names_durations_counts_and_switch_keys() {
        val metrics = PenPathMetrics()
        assertEquals("pen_path_timings v1\nswitches=none\n", metrics.report(emptyList()))
        metrics.recordMillis(PenTiming.CanvasSave, 3); metrics.recordMillis(PenTiming.CanvasSave, 12)
        metrics.record(PenTiming.RemoteEcho, 61_250)
        val text = metrics.report(listOf("remoteExactTimes", "canvasQuickRefresh"))
        assertEquals("pen_path_timings v1\n" +
            "canvas_save n=2 p50_ms=3.0 p95_ms=12.0 max_ms=12.0\n" +
            "remote_echo n=1 p50_ms=61.3 p95_ms=61.3 max_ms=61.3\n" +
            "switches=canvasQuickRefresh,remoteExactTimes\n", text)
        val line = Regex("[a-z_]+ n=[0-9]+ p50_ms=[0-9]+\\.[0-9] p95_ms=[0-9]+\\.[0-9] max_ms=[0-9]+\\.[0-9]")
        val lines = text.trimEnd().lines()
        assertEquals("pen_path_timings v1", lines.first())
        assertTrue(lines.last().matches(Regex("switches=[A-Za-z,]+")))
        for (row in lines.drop(1).dropLast(1)) assertTrue(row, row.matches(line))
        assertEquals(PenTiming.entries.size, PenTiming.entries.map { it.key }.toSet().size)
        for (timing in PenTiming.entries) assertTrue(timing.key.matches(Regex("[a-z]+(_[a-z]+)+")) && timing.label.isNotBlank())
        assertEquals("0.0", PenPathMetrics.ms(0)); assertEquals("0.1", PenPathMetrics.ms(50)); assertEquals("25.0", PenPathMetrics.ms(25_000))
    }
}
