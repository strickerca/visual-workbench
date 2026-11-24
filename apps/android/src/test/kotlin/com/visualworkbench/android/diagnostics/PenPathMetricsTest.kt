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
    /** The same ink is drawn again for a hover, a status change, after a saved
     * frame and while a finished stroke waits to be saved. Those frames show no
     * new pen sample and must not be reported as pen-to-ink time, or a slow
     * save would look like slow ink. */
    @Test fun a_frame_that_shows_no_new_pen_sample_is_not_timed() {
        val metrics = PenPathMetrics()
        metrics.inkDrawn(1_004, 1_000, 1_012, 3_000_000); metrics.frameReady(1_004, 1_014)
        // The stroke is finished and stays on screen for 400 ms while it is saved; it is redrawn four times.
        for (redrawn in listOf(1_060L, 1_150L, 1_290L, 1_410L)) { metrics.inkDrawn(1_004, 1_000, redrawn, 2_000_000); metrics.frameReady(1_004, redrawn + 2) }
        assertEquals(TimingSummary(1, 1, 8_000, 8_000, 8_000), metrics.summary(PenTiming.CanvasInkDrawn))
        assertEquals(TimingSummary(1, 1, 10_000, 10_000, 10_000), metrics.summary(PenTiming.CanvasFrameReady))
        assertEquals(TimingSummary(1, 1, 3_000, 3_000, 3_000), metrics.summary(PenTiming.CanvasDrawTime))
        assertEquals(TimingSummary(1, 1, 12_000, 12_000, 12_000), metrics.summary(PenTiming.CanvasFirstInk))
        // The next stroke's first sample is timed again, and a hover frame between two strokes is not.
        metrics.inkDrawn(1_004, 1_000, 1_900, 1_000_000)
        metrics.inkDrawn(2_005, 2_000, 2_011, 1_000_000); metrics.frameReady(2_005, 2_012)
        assertEquals(2, metrics.summary(PenTiming.CanvasInkDrawn).count); assertEquals(8_000L, metrics.summary(PenTiming.CanvasInkDrawn).maxMicros)
        assertEquals(2, metrics.summary(PenTiming.CanvasFrameReady).count); assertEquals(10_000L, metrics.summary(PenTiming.CanvasFrameReady).maxMicros)
        assertEquals(2, metrics.summary(PenTiming.CanvasFirstInk).count)
        // A frame without wet ink is still nothing to time.
        metrics.inkDrawn(0, 0, 3_000, 1_000_000); metrics.frameReady(0, 3_001)
        assertEquals(2, metrics.summary(PenTiming.CanvasInkDrawn).count)
    }
    @Test fun recording_from_several_threads_loses_nothing_and_takes_no_lock() {
        val window = TimingWindow(capacity = 4_096)
        val threads = (1..4).map { thread -> Thread { for (value in 1..1_000) window.record((thread * 10_000 + value).toLong()) } }
        threads.forEach { it.start() }; threads.forEach { it.join() }
        val all = window.summary()
        assertEquals(4_000, all.count); assertEquals(4_000L, all.total); assertEquals(41_000L, all.maxMicros)
        // Nothing in the recording path may synchronise: it runs on the drawing thread.
        for (method in TimingWindow::class.java.declaredMethods + PenPathMetrics::class.java.declaredMethods)
            assertFalse(method.name, java.lang.reflect.Modifier.isSynchronized(method.modifiers))
    }
    /** Privacy rule: the copied text holds static names, durations and counts only. */
    @Test fun the_copied_text_contains_only_fixed_names_durations_counts_and_switch_keys() {
        val metrics = PenPathMetrics()
        assertEquals("pen_path_timings v1\nswitches=none\n", metrics.report(emptyList()))
        metrics.recordMillis(PenTiming.CanvasSave, 3); metrics.recordMillis(PenTiming.CanvasSave, 12)
        metrics.record(PenTiming.RemoteEcho, 61_250)
        val text = metrics.report(listOf("remoteExactTimes", "remoteQuickFrames"))
        assertEquals("pen_path_timings v1\n" +
            "canvas_save n=2 p50_ms=3.0 p95_ms=12.0 max_ms=12.0\n" +
            "remote_echo n=1 p50_ms=61.3 p95_ms=61.3 max_ms=61.3\n" +
            "switches=remoteExactTimes,remoteQuickFrames\n", text)
        val line = Regex("[a-z_]+ n=[0-9]+ p50_ms=[0-9]+\\.[0-9] p95_ms=[0-9]+\\.[0-9] max_ms=[0-9]+\\.[0-9]")
        val lines = text.trimEnd().lines()
        assertEquals("pen_path_timings v1", lines.first())
        assertTrue(lines.last().matches(Regex("switches=[A-Za-z,]+")))
        for (row in lines.drop(1).dropLast(1)) assertTrue(row, row.matches(line))
        assertEquals(PenTiming.entries.size, PenTiming.entries.map { it.key }.toSet().size)
        for (timing in PenTiming.entries) assertTrue(timing.key.matches(Regex("[a-z]+(_[a-z]+)+")) && timing.label.isNotBlank())
        assertEquals("0.0", PenPathMetrics.ms(0)); assertEquals("0.1", PenPathMetrics.ms(50)); assertEquals("25.0", PenPathMetrics.ms(25_000))
    }
    /** A pen-down that asked for ink is counted as drawn or as refused with its
     * reason, so a person or a test can see that none was lost. */
    @Test fun pen_downs_for_ink_are_counted_as_drawn_or_refused_by_reason() {
        val metrics = PenPathMetrics()
        assertEquals(PenDownCounts(0, emptyMap()), metrics.penDowns())
        repeat(412) { metrics.penDown(null) }
        assertEquals(PenDownCounts(412, emptyMap()), metrics.penDowns())
        assertEquals(0L, metrics.penDowns().refusedTotal)
        assertEquals("Pen-downs for ink: 412 drawn, none refused", PenPathMetrics.penDownsLine(metrics.penDowns()))
        assertEquals("pen_path_timings v1\npen_downs accepted=412 refused=0\nswitches=none\n", metrics.report(emptyList()))
        metrics.penDown(PenRefusal.Modal); metrics.penDown(PenRefusal.Backlog); metrics.penDown(PenRefusal.Modal)
        val counts = metrics.penDowns()
        assertEquals(412L, counts.accepted); assertEquals(3L, counts.refusedTotal)
        assertEquals(mapOf(PenRefusal.Backlog to 1L, PenRefusal.Modal to 2L), counts.refused)
        assertEquals("Pen-downs for ink: 412 drawn, 3 refused (too much ink still unsaved: 1; a dialog, panel or another screen was open: 2)",
            PenPathMetrics.penDownsLine(counts))
        metrics.recordMillis(PenTiming.CanvasSave, 3)
        assertEquals("pen_path_timings v1\n" +
            "canvas_save n=1 p50_ms=3.0 p95_ms=3.0 max_ms=3.0\n" +
            "pen_downs accepted=412 refused=3 backlog=1 modal=2\n" +
            "switches=none\n", metrics.report(emptyList()))
        // Fixed names only, as for the timings; the reasons add up to the total.
        assertEquals(PenRefusal.entries.size, PenRefusal.entries.map { it.key }.toSet().size)
        for (reason in PenRefusal.entries) assertTrue(reason.key.matches(Regex("[a-z]+(_[a-z]+)*")) && reason.label.isNotBlank())
        metrics.clear()
        assertEquals(PenDownCounts(0, emptyMap()), metrics.penDowns())
        assertEquals("pen_path_timings v1\nswitches=none\n", metrics.report(emptyList()))
    }
}
