package com.visualworkbench.android.editor

import kotlinx.coroutines.Deferred
import org.junit.Assert.*
import org.junit.Test

class CanvasGenerationsTest {
    /** The original completion cause, as handlers observe it (no stack-trace recovery copy). */
    private fun failureOf(value: Deferred<*>): Throwable? { var cause: Throwable? = null; value.invokeOnCompletion { cause = it }; return cause }

    @Test fun replacementKeepsAtMostTwoGenerationsAndNeverAllocatesWhileAReleaseSettles() {
        val generations = CanvasGenerations()
        assertTrue(generations.canCreate(hasCurrent = false))
        assertFalse(generations.canCreate(hasCurrent = true))
        // Current presented generation: one successor may be prepared beside it.
        assertTrue(generations.canReplace(hasCurrent = true, hasKept = false, giant = false))
        // A kept predecessor already exists: no second replacement (no third pool).
        assertFalse(generations.canReplace(hasCurrent = true, hasKept = true, giant = false))
        val kept = generations.beginRelease()
        assertTrue(generations.releasing)
        // While the predecessor releases, neither a fresh nor a replacement generation is allocated.
        assertFalse(generations.canCreate(hasCurrent = false))
        assertFalse(generations.canReplace(hasCurrent = true, hasKept = false, giant = false))
        kept.complete(Unit); generations.settled(kept)
        assertFalse(generations.releasing)
        assertTrue(generations.canCreate(hasCurrent = false))
    }

    @Test fun giantModeNeverReplacesInPlace() {
        assertFalse(CanvasGenerations().canReplace(hasCurrent = true, hasKept = false, giant = true))
        assertFalse(CanvasGenerations().canReplace(hasCurrent = false, hasKept = false, giant = false))
    }

    @Test fun joinedWaitsForEveryOverlappingRelease() {
        val generations = CanvasGenerations()
        assertTrue(generations.joined().isCompleted)
        val kept = generations.beginRelease(); val current = generations.beginRelease()
        assertEquals(2, generations.releaseCount)
        val joined = generations.joined()
        kept.complete(Unit); generations.settled(kept)
        assertFalse(joined.isCompleted)
        current.complete(Unit); generations.settled(current)
        assertTrue(joined.isCompleted); assertFalse(joined.isCancelled)
        // Nothing outstanding: the most recent release is exposed again.
        assertSame(current, generations.joined())
    }

    @Test fun joinedFailsWhenAnyReleaseFailsAndKeepsTheLastFailureVisible() {
        val generations = CanvasGenerations()
        val kept = generations.beginRelease(); val current = generations.beginRelease()
        val joined = generations.joined()
        val failure = IllegalStateException("reader retained")
        current.completeExceptionally(failure); generations.settled(current)
        assertTrue(joined.isCompleted)
        assertSame(failure, failureOf(joined))
        kept.complete(Unit); generations.settled(kept)
        assertSame(failure, failureOf(generations.joined()))
    }

    @Test fun singleOutstandingReleaseIsReturnedDirectly() {
        val generations = CanvasGenerations()
        val only = generations.beginRelease()
        assertSame(only, generations.joined())
    }
}

class CanvasRedrawHandshakeTest {
    private class Scheduler {
        val scheduled = mutableListOf<Pair<Runnable, Long>>()
        fun schedule(task: Runnable, delay: Long) { scheduled += task to delay }
        fun unschedule(task: Runnable) { scheduled.removeAll { it.first === task } }
    }

    @Test fun presentedCanvasFinishesImmediately() {
        val scheduler = Scheduler(); var finished = 0
        val handshake = CanvasRedrawHandshake(scheduler::schedule, scheduler::unschedule, 750)
        handshake.request({ finished++ }, presented = true)
        assertEquals(1, finished); assertEquals(0, handshake.waiting); assertTrue(scheduler.scheduled.isEmpty())
    }

    @Test fun unpresentedCanvasHoldsUntilFirstCommittedFrame() {
        val scheduler = Scheduler(); var finished = 0
        val handshake = CanvasRedrawHandshake(scheduler::schedule, scheduler::unschedule, 750)
        handshake.request({ finished++ }, presented = false)
        handshake.request({ finished++ }, presented = false)
        assertEquals(0, finished); assertEquals(2, handshake.waiting)
        assertEquals(listOf(750L), scheduler.scheduled.map { it.second })
        handshake.finish()
        assertEquals(2, finished); assertEquals(0, handshake.waiting); assertTrue(scheduler.scheduled.isEmpty())
        handshake.finish()
        assertEquals(2, finished)
    }

    @Test fun boundedFallbackReleasesAHeldRedraw() {
        val scheduler = Scheduler(); var finished = 0
        val handshake = CanvasRedrawHandshake(scheduler::schedule, scheduler::unschedule, 750)
        handshake.request({ finished++ }, presented = false)
        scheduler.scheduled.single().first.run()
        assertEquals(1, finished); assertEquals(0, handshake.waiting)
    }
}
