package com.visualworkbench.android.editor

import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch

class CanvasLifeRingTest {
    private class Taken(val kind: Int, val surface: Int, val timeMs: Long, val values: List<Long>)

    private fun CanvasLifeRing.take(): List<Taken> {
        val taken = mutableListOf<Taken>()
        drain { kind, surface, timeMs, values -> taken += Taken(kind, surface, timeMs, values.toList()) }
        return taken
    }

    /** A record whose twelve numbers are [first], [first] + 1, ... */
    private fun CanvasLifeRing.put(kind: Int, surface: Int = 1, timeMs: Long = 0, first: Long = 0) =
        write(kind, surface, timeMs, first, first + 1, first + 2, first + 3, first + 4, first + 5, first + 6, first + 7,
            first + 8, first + 9, first + 10, first + 11)

    @Test fun recordsComeOutOnceInTheOrderTheyWereWritten() {
        val ring = CanvasLifeRing(8)
        assertEquals(0, ring.drain { _, _, _, _ -> fail("nothing was written") })
        ring.put(3, surface = 2, timeMs = 100, first = 10); ring.put(4, surface = 5, timeMs = 101, first = -20)
        val taken = ring.take()
        assertEquals(listOf(3, 4), taken.map { it.kind }); assertEquals(listOf(2, 5), taken.map { it.surface })
        assertEquals(listOf(100L, 101L), taken.map { it.timeMs })
        assertEquals((10L..21L).toList(), taken[0].values); assertEquals((-20L..-9L).toList(), taken[1].values)
        assertTrue("a record is delivered once", ring.take().isEmpty())
        ring.put(7)
        assertEquals(listOf(7), ring.take().map { it.kind })
        assertEquals(0L, ring.takeDropped())
    }

    @Test fun aFullRingKeepsTheNewestRecordsAndCountsTheRest() {
        val ring = CanvasLifeRing(4)
        for (kind in 1..10) ring.put(kind, first = kind * 100L)
        val taken = ring.take()
        assertEquals(listOf(7, 8, 9, 10), taken.map { it.kind })
        assertEquals(700L, taken.first().values.first())
        assertEquals(6L, ring.takeDropped()); assertEquals(0L, ring.takeDropped())
        ring.put(11); ring.put(12)
        assertEquals(listOf(11, 12), ring.take().map { it.kind }); assertEquals(0L, ring.takeDropped())
    }

    /** Four writers into a small ring while it is drained: every record delivered is
     * whole (each of its numbers belongs to it), each writer's records come out in
     * the order it wrote them, and every record is either delivered or counted. */
    @Test fun writersOnSeveralThreadsNeverDeliverAMixedOrRepeatedRecord() {
        val ring = CanvasLifeRing(64)
        val writers = 4; val each = 20_000
        val last = LongArray(writers + 1) { -1 }
        var delivered = 0L
        val sink = CanvasLifeRing.Sink { kind, surface, timeMs, values ->
            assertEquals(kind, surface)
            for (index in values.indices) assertEquals(timeMs * 13 + index + kind, values[index])
            assertTrue("writer $kind went back from ${last[kind]} to $timeMs", timeMs > last[kind])
            last[kind] = timeMs; delivered++
        }
        val start = CountDownLatch(1)
        val threads = (1..writers).map { writer -> Thread {
            start.await()
            for (sequence in 0 until each) {
                val base = sequence * 13L + writer
                ring.write(writer, writer, sequence.toLong(), base, base + 1, base + 2, base + 3, base + 4, base + 5, base + 6,
                    base + 7, base + 8, base + 9, base + 10, base + 11)
            }
        } }
        threads.forEach { it.start() }; start.countDown()
        while (threads.any { it.isAlive }) ring.drain(sink)
        threads.forEach { it.join() }
        ring.drain(sink)
        assertEquals(writers.toLong() * each, delivered + ring.takeDropped())
        assertTrue(delivered >= 64)
    }

    @Test(expected = IllegalArgumentException::class) fun needsRoomForOneRecord() { CanvasLifeRing(0) }
}

class CanvasLifeEventTest {
    private fun line(event: CanvasLifeEvent, vararg numbers: Long, surface: Int = 3, timeMs: Long = 1_234): String =
        formatCanvasLifeRecord(event.ordinal, surface, timeMs, LongArray(CanvasLifeRing.VALUES).also { numbers.copyInto(it) })

    @Test fun aSavedFrameReadsAsBefore() {
        assertEquals("t=1234 surface=3 draw gen=2 frame=5 rev=41 scene_view=1080x1920 buffer=1080x1920 requests=1 " +
            "in_flight=1 newest_rev=42 ink_frames=0 closed=false",
            line(CanvasLifeEvent.Draw, 2, 5, 41, 1080, 1920, 1080, 1920, 1, 1, 42, 0, 0))
        assertEquals("t=1234 surface=3 frame_ready gen=2 frame=5 started=6 visible=true new_layer=false resize_redraw=true " +
            "in_flight=1 request_pending=false drawn_rev=41 newest_rev=42",
            line(CanvasLifeEvent.FrameReady, 2, 5, 6, 1, 0, 1, 1, 0, 41, 42))
    }

    @Test fun sizesWithoutASceneReadNone() {
        assertEquals("t=1234 surface=3 scene_without_generation rev=7 scene_view=none",
            line(CanvasLifeEvent.SceneWithoutGeneration, 7, -1, -1))
        assertEquals("t=9 surface=1 surface_changed size=1080x1500 changed=true",
            line(CanvasLifeEvent.SurfaceChanged, 1080, 1500, 1, surface = 1, timeMs = 9))
    }

    @Test fun wordsAreCarriedByTheirPosition() {
        assertEquals("t=1234 surface=3 retire reason=WindowDetached current=4 kept=0",
            line(CanvasLifeEvent.Retire, CanvasRetirementReason.WindowDetached.ordinal.toLong(), 4, 0))
        assertEquals("t=1234 surface=3 resize action=Replace size=1080x1400 current=2 current_presented=true kept=0 releases=0",
            line(CanvasLifeEvent.Resize, CanvasReplacement.ResizeAction.Replace.ordinal.toLong(), 1080, 1400, 2, 1, 0, 0))
        assertEquals("t=1234 surface=3 redraw_finish cause=presented released=2",
            line(CanvasLifeEvent.RedrawFinish, CanvasLifeWords.redrawCauses.indexOf("presented").toLong(), 2))
        assertEquals("t=1234 surface=3 frame_request gen=1 cause=newer_scene rev=9 scene_view=800x600 result=draw_in_flight in_flight=1",
            line(CanvasLifeEvent.FrameRequest, 1, 2, 9, 800, 600, 4, 1))
    }

    @Test fun anAbsentRetirementIsNoneAndAnUnknownWordIsItsNumber() {
        assertEquals("t=1234 surface=3 generation_waits reason=controller_gate releases=0 awaited_retirement=None awaited_milestone=None",
            line(CanvasLifeEvent.GenerationWaits, CanvasLifeWords.waitReasons.indexOf("controller_gate").toLong(), 0, 0, 0))
        assertEquals("t=1234 surface=3 generation_waits reason=other releases=1 awaited_retirement=SurfaceChanged " +
            "awaited_milestone=ReaderClosed",
            line(CanvasLifeEvent.GenerationWaits, 6, 1, CanvasRetirementReason.SurfaceChanged.ordinal + 1L,
                CanvasRetirementMilestone.ReaderClosed.ordinal + 1L))
        assertEquals("t=1234 surface=3 redraw_finish cause=-1 released=1", line(CanvasLifeEvent.RedrawFinish, -1, 1))
        assertEquals("t=1 surface=2 unknown_record kind=999", formatCanvasLifeRecord(999, 2, 1, LongArray(CanvasLifeRing.VALUES)))
    }

    @Test fun everyEventFitsOneRecordAndSaysWhatHappenedInItsOwnWord() {
        for (event in CanvasLifeEvent.entries) assertTrue("${event.name} ${event.arity}", event.arity <= CanvasLifeRing.VALUES)
        assertEquals(CanvasLifeEvent.entries.size, CanvasLifeEvent.entries.map { it.word }.toSet().size)
        assertEquals(12, CanvasLifeEvent.Draw.arity); assertEquals(0, CanvasLifeEvent.Attach.arity)
        // Each record's numbers are all used: one number more would not appear in its line.
        for (event in CanvasLifeEvent.entries) {
            val numbers = LongArray(CanvasLifeRing.VALUES) { 900_000L + it }
            val text = formatCanvasLifeRecord(event.ordinal, 1, 0, numbers)
            if (event.arity < CanvasLifeRing.VALUES) assertFalse(event.name, text.contains("${900_000L + event.arity}"))
        }
    }
}

class CanvasLifeHistoryTest {
    @Test fun keepsOnlyTheNewestLinesOldestFirst() {
        val history = CanvasLifeHistory(3)
        assertTrue(history.snapshot().isEmpty())
        for (line in 1..5) history.add("line $line")
        assertEquals(listOf("line 3", "line 4", "line 5"), history.snapshot())
    }

    @Test fun aSnapshotDoesNotChangeWhenLinesFollow() {
        val history = CanvasLifeHistory(2)
        history.add("first"); val taken = history.snapshot(); history.add("second"); history.add("third")
        assertEquals(listOf("first"), taken)
        assertEquals(listOf("second", "third"), history.snapshot())
    }

    @Test fun linesFromSeveralThreadsAreAllKept() {
        val history = CanvasLifeHistory(8_000)
        val writers = (1..4).map { writer -> Thread { repeat(1_000) { history.add("writer $writer line $it") } } }
        writers.forEach { it.start() }; writers.forEach { it.join() }
        val lines = history.snapshot()
        assertEquals(4_000, lines.size); assertEquals(4_000, lines.toSet().size)
    }

    @Test(expected = IllegalArgumentException::class) fun needsRoomForOneLine() { CanvasLifeHistory(0) }
}

class CanvasLifeBudgetTest {
    private fun CanvasLifeBudget.taken() = listOf(takenRequests, takenGranted, takenFrames, takenInk)

    @Test fun aNewGenerationRecordsItsFirstFramesAndRequestsThenCounts() {
        val budget = CanvasLifeBudget(frames = 3, requests = 2, summaryMs = 1_000)
        assertEquals(listOf(true, true, true, false, false), (1..5).map(budget::frameWritten))
        assertEquals(listOf(true, true, false, false), List(4) { budget.request(granted = true) })
    }

    @Test fun hoverAtTwoHundredSamplesASecondRecordsOneSummaryASecond() {
        val budget = CanvasLifeBudget(frames = 8, requests = 16, summaryMs = 1_000)
        repeat(16) { budget.request(granted = true) }
        repeat(8) { budget.frameWritten(it + 1) }
        val summaries = mutableListOf<List<Int>>()
        var frame = 8
        for (sample in 0 until 600) { // three seconds, one sample every 5 ms
            val now = 10_000L + sample * 5
            if (!budget.request(granted = sample % 2 == 0) && budget.summaryDue(now)) summaries += budget.taken()
            if (sample % 2 == 0 && !budget.frameWritten(++frame)) { budget.quietFrame(); if (budget.summaryDue(now)) summaries += budget.taken() }
        }
        assertEquals(3, summaries.size)
        assertEquals(listOf(1, 1, 0, 0), summaries.first())
        assertEquals(listOf(200, 100, 100), summaries[1].take(3))
    }

    @Test fun aLifecycleEventRecordsTheNextFramesAgainAndHandsOverWhatWasCounted() {
        val budget = CanvasLifeBudget(frames = 2, requests = 1, summaryMs = 1_000)
        budget.request(granted = true)
        assertFalse(budget.request(granted = false)); budget.quietFrame(); budget.inkFrame(); budget.inkFrame()
        assertFalse(budget.frameWritten(3))
        assertTrue(budget.again(started = 2, nowMs = 50)); assertEquals(listOf(1, 0, 1, 2), budget.taken())
        assertTrue(budget.frameWritten(3)); assertTrue(budget.frameWritten(4)); assertFalse(budget.frameWritten(5))
        assertTrue(budget.request(granted = true))
        assertFalse("nothing new was counted", budget.again(started = 4, nowMs = 60))
        assertTrue("the event re-opened the requests", budget.request(granted = false))
    }

    /** A frame's start and completion are numbered in the same order, so a frame
     * whose start was recorded also has its completion recorded, even when the next
     * event arrives between them. */
    @Test fun aFrameWhoseStartWasRecordedHasItsCompletionRecorded() {
        val budget = CanvasLifeBudget(frames = 1, requests = 0, summaryMs = 1_000)
        assertTrue(budget.frameWritten(1))
        assertFalse(budget.again(started = 1, nowMs = 0))
        assertTrue(budget.frameWritten(1)); assertTrue(budget.frameWritten(2))
    }

    @Test fun inkFramesAreCountedOnceWhereverTheyAreRecorded() {
        val budget = CanvasLifeBudget(frames = 1, requests = 0, summaryMs = 1_000)
        repeat(5) { budget.inkFrame() }
        assertEquals(5, budget.takeInk()); assertEquals(0, budget.takeInk())
        budget.inkFrame()
        assertTrue(budget.summaryDue(0)); assertEquals(listOf(0, 0, 0, 1), budget.taken())
        assertFalse(budget.summaryDue(5_000))
    }

    /** The one-second gap follows the last summary even when that summary was taken at uptime 0. */
    @Test fun theGapFollowsTheLastSummaryWhateverItsTime() {
        val budget = CanvasLifeBudget(frames = 0, requests = 0, summaryMs = 1_000)
        budget.quietFrame(); assertTrue(budget.summaryDue(0))
        budget.quietFrame(); assertFalse(budget.summaryDue(999))
        assertTrue(budget.summaryDue(1_000)); assertEquals(listOf(0, 0, 1, 0), budget.taken())
    }

    @Test(expected = IllegalArgumentException::class) fun aSummaryGapMustBePositive() { CanvasLifeBudget(1, 1, 0) }
}
