package com.visualworkbench.android.editor

import kotlinx.coroutines.Deferred
import org.junit.Assert.*
import org.junit.Test

/** The original completion cause, as handlers observe it (no stack-trace recovery copy). */
private fun failureOf(value: Deferred<*>): Throwable? { var cause: Throwable? = null; value.invokeOnCompletion { cause = it }; return cause }

class CanvasGenerationsTest {
    @Test fun noGenerationIsAllocatedWhileAReleaseSettles() {
        val generations = CanvasGenerations()
        assertTrue(generations.canCreate(hasCurrent = false))
        assertFalse(generations.canCreate(hasCurrent = true))
        val kept = generations.beginRelease()
        assertTrue(generations.releasing)
        assertFalse(generations.canCreate(hasCurrent = false))
        kept.complete(Unit); generations.settled(kept)
        assertFalse(generations.releasing)
        assertTrue(generations.canCreate(hasCurrent = false))
    }

    @Test fun nothingReleasedJoinsImmediately() {
        val joined = CanvasGenerations().joined()
        assertTrue(joined.isCompleted); assertNull(failureOf(joined))
    }

    @Test fun joinedWaitsForEveryOverlappingRelease() {
        val generations = CanvasGenerations()
        val kept = generations.beginRelease(); val current = generations.beginRelease()
        assertEquals(2, generations.releaseCount)
        val joined = generations.joined()
        kept.complete(Unit); generations.settled(kept)
        assertFalse(joined.isCompleted)
        current.complete(Unit); generations.settled(current)
        assertTrue(joined.isCompleted); assertNull(failureOf(joined))
    }

    @Test fun detachDuringOverlapWaitsForAllThenFailsWithTheFirstFailure() {
        val generations = CanvasGenerations()
        // A kept generation is still releasing when the window detaches the current one.
        val kept = generations.beginRelease()
        val detached = generations.beginRelease()
        val joined = generations.joined()
        val first = IllegalStateException("kept reader retained")
        kept.completeExceptionally(first); generations.settled(kept)
        assertFalse("must not fail while another release still runs", joined.isCompleted)
        val second = IllegalStateException("detached reader retained")
        detached.completeExceptionally(second); generations.settled(detached)
        assertTrue(joined.isCompleted)
        assertSame(first, failureOf(joined))
    }

    @Test fun aLateJoinerAlsoWaitsForReleasesBegunAfterIt() {
        val generations = CanvasGenerations()
        val kept = generations.beginRelease()
        val joined = generations.joined()
        val current = generations.beginRelease()
        kept.complete(Unit); generations.settled(kept)
        assertFalse(joined.isCompleted)
        current.complete(Unit); generations.settled(current)
        assertTrue(joined.isCompleted)
    }

    @Test fun aSettledFailureStaysVisibleUntilTheNextBatchBegins() {
        val generations = CanvasGenerations()
        val failed = generations.beginRelease()
        val failure = IllegalStateException("reader retained")
        failed.completeExceptionally(failure); generations.settled(failed)
        assertSame(failure, failureOf(generations.joined()))
        assertSame(failure, failureOf(generations.joined()))
        val next = generations.beginRelease()
        val joined = generations.joined()
        assertFalse(joined.isCompleted)
        next.complete(Unit); generations.settled(next)
        assertTrue(joined.isCompleted); assertNull(failureOf(joined))
    }

    @Test fun incompleteOrUnknownTokensNeverSettle() {
        val generations = CanvasGenerations()
        val stuck = generations.beginRelease()
        generations.settled(stuck) // Not completed: its release callback never arrived.
        assertTrue(generations.releasing)
        assertFalse(generations.joined().isCompleted)
        val foreign = CanvasGenerations().beginRelease().also { it.complete(Unit) }
        generations.settled(foreign)
        assertEquals(1, generations.releaseCount)
    }
}

class CanvasReplacementTest {
    private class Gen(val name: String) { override fun toString() = name }

    @Test fun missingRendererAndGiantModeKeepBreakBeforeMake() {
        val replacement = CanvasReplacement<Gen>()
        assertEquals(CanvasReplacement.ResizeAction.RetireAll, replacement.resizeAction(hasCurrent = false, giant = false, releasing = false))
        assertEquals(CanvasReplacement.ResizeAction.RetireAll, replacement.resizeAction(hasCurrent = true, giant = true, releasing = false))
        assertFalse(replacement.replaceDeferred)
    }

    @Test fun firstResizeReplacesAndKeepsUntilSuccessorPresentedAndKeptRedrawn() {
        val replacement = CanvasReplacement<Gen>(); val old = Gen("old")
        assertEquals(CanvasReplacement.ResizeAction.Replace, replacement.resizeAction(hasCurrent = true, giant = false, releasing = false))
        replacement.keep(old, awaitRedraw = true)
        assertNull("the kept redraw still carries SurfaceView's completion", replacement.successorPresented())
        assertSame(old, replacement.kept)
        assertSame(old, replacement.keptRedrawn(old))
        assertNull(replacement.kept)
    }

    @Test fun keptRedrawBeforeSuccessorPresentStillWaitsForThePresent() {
        val replacement = CanvasReplacement<Gen>(); val old = Gen("old")
        replacement.keep(old, awaitRedraw = true)
        assertNull(replacement.keptRedrawn(old))
        assertSame(old, replacement.kept)
        assertSame(old, replacement.successorPresented())
    }

    @Test fun noPendingRedrawRetiresOnPresent() {
        val replacement = CanvasReplacement<Gen>(); val old = Gen("old")
        replacement.keep(old, awaitRedraw = false)
        assertSame(old, replacement.successorPresented())
    }

    @Test fun redrawOfAnotherGenerationIsIgnored() {
        val replacement = CanvasReplacement<Gen>(); val old = Gen("old")
        replacement.keep(old, awaitRedraw = true)
        assertNull(replacement.keptRedrawn(Gen("successor")))
        assertNull(replacement.successorPresented())
        assertSame(old, replacement.kept)
    }

    @Test fun fallbackRetiresOnlyAfterTheSuccessorPresented() {
        val replacement = CanvasReplacement<Gen>(); val old = Gen("old")
        replacement.keep(old, awaitRedraw = true)
        assertNull(replacement.fallback())
        assertNull(replacement.successorPresented())
        assertSame(old, replacement.fallback())
        assertNull(replacement.fallback())
    }

    @Test fun resizeBeforeSuccessorPresentsDefersAndRearmsTheKeptRedraw() {
        val replacement = CanvasReplacement<Gen>(); val old = Gen("old")
        replacement.keep(old, awaitRedraw = true)
        assertNull(replacement.keptRedrawn(old))
        assertEquals(CanvasReplacement.ResizeAction.Defer, replacement.resizeAction(hasCurrent = true, giant = false, releasing = false))
        assertTrue(replacement.replaceDeferred)
        // The earlier redraw no longer counts: the kept generation resizes again.
        assertNull(replacement.successorPresented())
        assertSame(old, replacement.keptRedrawn(old))
    }

    @Test fun resizesDuringASettlingReleaseCoalesceIntoOneReplacement() {
        val replacement = CanvasReplacement<Gen>()
        repeat(3) {
            assertEquals(CanvasReplacement.ResizeAction.Defer, replacement.resizeAction(hasCurrent = true, giant = false, releasing = true))
        }
        assertFalse(replacement.replaceDue(releasing = true))
        assertTrue(replacement.replaceDue(releasing = false))
        assertFalse(replacement.replaceDue(releasing = false))
    }

    @Test fun deferredReplacementWaitsForTheKeptGenerationToLeave() {
        val replacement = CanvasReplacement<Gen>(); val old = Gen("old")
        replacement.keep(old, awaitRedraw = false)
        replacement.resizeAction(hasCurrent = true, giant = false, releasing = false)
        assertFalse(replacement.replaceDue(releasing = false))
        assertSame(old, replacement.keptRedrawn(old) ?: replacement.successorPresented())
        assertTrue(replacement.replaceDue(releasing = false))
    }

    @Test fun refusedSuccessorRestoresTheKeptGeneration() {
        val replacement = CanvasReplacement<Gen>(); val old = Gen("old")
        replacement.keep(old, awaitRedraw = true)
        assertSame(old, replacement.restore())
        assertNull(replacement.kept)
        assertNull(replacement.successorPresented())
    }

    @Test fun fullRetirementTakesTheKeptGenerationAndClearsDeferral() {
        val replacement = CanvasReplacement<Gen>(); val old = Gen("old")
        replacement.keep(old, awaitRedraw = true)
        replacement.resizeAction(hasCurrent = true, giant = false, releasing = false)
        replacement.clearDeferred()
        assertSame(old, replacement.take())
        assertNull(replacement.take())
        assertFalse(replacement.replaceDue(releasing = false))
    }

    @Test(expected = IllegalStateException::class) fun onlyOneReplacementAtATime() {
        val replacement = CanvasReplacement<Gen>()
        replacement.keep(Gen("a"), awaitRedraw = false)
        replacement.keep(Gen("b"), awaitRedraw = false)
    }
}

class CanvasRedrawHandshakeTest {
    private class Scheduler {
        val scheduled = mutableListOf<Pair<Runnable, Long>>()
        var scheduleCalls = 0
        fun schedule(task: Runnable, delay: Long) { scheduleCalls++; scheduled += task to delay }
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

    @Test fun laterRequestsNeverRestartTheFallback() {
        val scheduler = Scheduler()
        val handshake = CanvasRedrawHandshake(scheduler::schedule, scheduler::unschedule, 750)
        repeat(4) { handshake.request({}, presented = false) }
        assertEquals(1, scheduler.scheduleCalls)
    }

    @Test fun boundedFallbackReleasesAHeldRedraw() {
        val scheduler = Scheduler(); var finished = 0
        val handshake = CanvasRedrawHandshake(scheduler::schedule, scheduler::unschedule, 750)
        handshake.request({ finished++ }, presented = false)
        scheduler.scheduled.single().first.run()
        assertEquals(1, finished); assertEquals(0, handshake.waiting)
    }
}
