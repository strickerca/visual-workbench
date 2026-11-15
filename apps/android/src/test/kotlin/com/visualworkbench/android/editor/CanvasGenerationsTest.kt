package com.visualworkbench.android.editor

import kotlinx.coroutines.CompletableDeferred
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

    @Test fun noGenerationIsCreatedWhileTheSurfaceReportsItsNewSize() {
        val generations = CanvasGenerations()
        assertFalse(generations.viewportChanging)
        val result = generations.whileViewportChanges {
            assertTrue(generations.viewportChanging)
            assertFalse(generations.canCreate(hasCurrent = false))
            // The owner may fit the page inside its size update: a report within a report.
            generations.whileViewportChanges { assertFalse(generations.canCreate(hasCurrent = false)) }
            assertFalse(generations.canCreate(hasCurrent = false))
            "reported"
        }
        assertEquals("reported", result)
        assertFalse(generations.viewportChanging)
        assertTrue(generations.canCreate(hasCurrent = false))
    }

    @Test fun aSizeReportThatFailsStillReopensCreation() {
        val generations = CanvasGenerations()
        val failure = IllegalStateException("the owner refused the size")
        try { generations.whileViewportChanges<Unit> { throw failure }; fail("the failure must reach the caller") }
        catch (error: IllegalStateException) { assertSame(failure, error) }
        assertFalse(generations.viewportChanging)
        assertTrue(generations.canCreate(hasCurrent = false))
    }

    /** The surface's sequence on a size change, with the owner's actual order: it
     * cancels the tool in hand, which publishes a scene still laid out for the old
     * size, then takes the new size and publishes the scene for it. */
    @Test fun aSuccessorStartsFromTheSceneForTheNewSizeNotFromOnePublishedOnTheWay() {
        val generations = CanvasGenerations()
        var latest = "old size"; var firstSceneOfSuccessor: String? = null
        fun ensure() { if (firstSceneOfSuccessor == null && generations.canCreate(hasCurrent = false)) firstSceneOfSuccessor = latest }
        fun publish(scene: String) { latest = scene; ensure() }
        generations.whileViewportChanges { publish("old size, tool cancelled"); publish("new size") }
        assertNull("nothing is created from inside the report", firstSceneOfSuccessor)
        ensure()
        assertEquals("new size", firstSceneOfSuccessor)
    }

    @Test fun aReleaseStillSettlingKeepsCreationClosedAfterTheSizeReport() {
        val generations = CanvasGenerations()
        val retired = generations.beginRelease()
        generations.whileViewportChanges { assertFalse(generations.canCreate(hasCurrent = false)) }
        assertFalse(generations.canCreate(hasCurrent = false))
        retired.complete(Unit); generations.settled(retired)
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

/** The order in which a surface takes a size, run on the real bookkeeping with a
 * stand-in for everything else. [Surface] follows CanvasSurface step for step
 * wherever a generation can be created (attach, surfaceCreated, surfaceChanged,
 * surfaceDestroyed, publish, ensureRenderer, resize). [Owner] follows the
 * controller wherever it publishes: cancelling the tool publishes, and taking a
 * viewport cancels the tool first, then takes the size, then publishes. The
 * owner outlives an activity, so after a recreation it still holds the camera
 * for the previous canvas. A scene is named by the viewport it was laid out for. */
class CanvasSizeChangeOrderTest {
    private class Owner(private var camera: String) {
        var sink: ((String) -> Unit)? = null
        fun publish() { sink?.invoke(camera) }
        fun cancelTool() = publish()
        fun viewport(size: String) { cancelTool(); camera = size; publish() }
    }

    private class Surface(private val owner: Owner, private val inputCancelHeld: Boolean = true) {
        val generations = CanvasGenerations()
        val replacement = CanvasReplacement<Int>()
        val actions = mutableListOf<CanvasReplacement.ResizeAction>()
        /** The scene each generation started from, in order of creation. */
        val firstScenes = mutableListOf<String>()
        var ownerAllowsCreation = true
        private var attached = false
        private var size: String? = null
        private var latest: String? = null
        private var current: Int? = null
        private val settling = mutableListOf<CompletableDeferred<Unit>>()

        fun attach() { attached = true; owner.sink = ::publish; owner.publish(); ensureRenderer() }
        fun surfaceCreated() { ensureRenderer() }
        fun surfaceChanged(newSize: String) {
            val changed = size != newSize
            size = newSize
            if (changed) {
                if (inputCancelHeld) generations.whileViewportChanges { cancelInput() } else cancelInput()
                resize(newSize)
            }
            ensureRenderer()
        }
        fun surfaceDestroyed() { size = null; cancelInput(); retireRenderer() }
        fun ownerAllowsCreationNow() { ownerAllowsCreation = true; ensureRenderer() }
        fun releasesSettle() {
            for (release in settling.toList()) { release.complete(Unit); generations.settled(release); ensureRenderer() }
            settling.clear()
        }

        private fun cancelInput() { owner.cancelTool() }
        private fun publish(scene: String) { latest = scene; if (current == null) ensureRenderer() }
        private fun ensureRenderer() {
            if (!attached || !ownerAllowsCreation || size == null || latest == null || !generations.canCreate(current != null)) return
            firstScenes += checkNotNull(latest); current = firstScenes.size
        }
        private fun retireRenderer() {
            replacement.clearDeferred()
            if (replacement.take() != null) settling += generations.beginRelease()
            if (current != null) { settling += generations.beginRelease(); current = null }
        }
        private fun reportViewport(newSize: String) { generations.whileViewportChanges { owner.viewport(newSize) } }
        private fun resize(newSize: String) {
            val action = replacement.resizeAction(current != null, giant = false, releasing = generations.releasing)
            actions += action
            when (action) {
                CanvasReplacement.ResizeAction.RetireAll -> { retireRenderer(); reportViewport(newSize) }
                CanvasReplacement.ResizeAction.Defer -> reportViewport(newSize)
                CanvasReplacement.ResizeAction.Replace -> {
                    replacement.keep(checkNotNull(current), awaitRedraw = true); current = null
                    reportViewport(newSize)
                    ensureRenderer()
                    if (current == null) current = replacement.restore()
                }
            }
        }
    }

    @Test fun theFirstSurfaceOfARecreatedActivityStartsOneGenerationFromTheSceneForItsOwnSize() {
        val surface = Surface(Owner(camera = "1080x1500"))
        surface.attach(); surface.surfaceCreated()
        assertTrue("nothing starts before the surface has a size", surface.firstScenes.isEmpty())
        surface.surfaceChanged("1500x1080")
        assertEquals(listOf(CanvasReplacement.ResizeAction.RetireAll), surface.actions)
        assertEquals(listOf("1500x1080"), surface.firstScenes)
        assertNull("nothing is kept: there was nothing to replace", surface.replacement.kept)
    }

    /** The order before the cancel was held: the size was assigned, the cancel
     * published the owner's old-size scene, and that started a first generation
     * which the size change then kept while it built a second. */
    @Test fun cancellingTheInputOutsideTheHoldStartedAGenerationForTheOldSizeAndThenASecond() {
        val surface = Surface(Owner(camera = "1080x1500"), inputCancelHeld = false)
        surface.attach(); surface.surfaceCreated(); surface.surfaceChanged("1500x1080")
        assertEquals(listOf(CanvasReplacement.ResizeAction.Replace), surface.actions)
        assertEquals(listOf("1080x1500", "1500x1080"), surface.firstScenes)
        assertEquals(1, surface.replacement.kept)
    }

    @Test fun aSurfaceThatComesBackStartsOneGenerationAgain() {
        val surface = Surface(Owner(camera = "1080x1500"))
        surface.attach(); surface.surfaceCreated(); surface.surfaceChanged("1080x1500")
        surface.surfaceDestroyed(); surface.releasesSettle()
        surface.surfaceCreated(); surface.surfaceChanged("1080x1500")
        assertEquals(listOf(CanvasReplacement.ResizeAction.RetireAll, CanvasReplacement.ResizeAction.RetireAll), surface.actions)
        assertEquals(listOf("1080x1500", "1080x1500"), surface.firstScenes)
        assertNull(surface.replacement.kept)
    }

    @Test fun aSurfaceThatComesBackBeforeItsReleaseSettledStartsOneGenerationOnceItHas() {
        val surface = Surface(Owner(camera = "1080x1500"))
        surface.attach(); surface.surfaceCreated(); surface.surfaceChanged("1080x1500")
        surface.surfaceDestroyed()
        surface.surfaceCreated(); surface.surfaceChanged("1080x1100")
        assertEquals(listOf("1080x1500"), surface.firstScenes)
        surface.releasesSettle()
        assertEquals(listOf("1080x1500", "1080x1100"), surface.firstScenes)
    }

    @Test fun aSurfaceWhoseOwnerStillWaitsForThePreviousCanvasStartsOneGenerationWhenAllowed() {
        val surface = Surface(Owner(camera = "1080x1500"))
        surface.ownerAllowsCreation = false
        surface.attach(); surface.surfaceCreated(); surface.surfaceChanged("1500x1080")
        assertTrue(surface.firstScenes.isEmpty())
        surface.ownerAllowsCreationNow()
        assertEquals(listOf("1500x1080"), surface.firstScenes)
    }

    @Test fun aLaterSizeChangeStillKeepsTheGenerationAndStartsOneSuccessorForTheNewSize() {
        val surface = Surface(Owner(camera = "1080x1500"))
        surface.attach(); surface.surfaceCreated(); surface.surfaceChanged("1080x1500")
        surface.surfaceChanged("1080x1100")
        assertEquals(listOf(CanvasReplacement.ResizeAction.RetireAll, CanvasReplacement.ResizeAction.Replace), surface.actions)
        assertEquals(listOf("1080x1500", "1080x1100"), surface.firstScenes)
        assertEquals(1, surface.replacement.kept)
    }

    @Test fun aSurfaceChangeThatKeepsTheSizeTouchesNothing() {
        val surface = Surface(Owner(camera = "1080x1500"))
        surface.attach(); surface.surfaceCreated(); surface.surfaceChanged("1080x1500")
        surface.surfaceChanged("1080x1500")
        assertEquals(1, surface.actions.size); assertEquals(1, surface.firstScenes.size)
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

    @Test fun onlyTheFallbackReportsItselfWithTheNumberOfRedrawsItReleased() {
        val scheduler = Scheduler(); val reported = mutableListOf<Int>(); var finished = 0
        val handshake = CanvasRedrawHandshake(scheduler::schedule, scheduler::unschedule, 750) { reported += it }
        handshake.request({ finished++ }, presented = false); handshake.request({ finished++ }, presented = false)
        scheduler.scheduled.single().first.run()
        assertEquals(listOf(2), reported); assertEquals(2, finished)
        // A first committed frame, a destroyed surface or a detached window finish without it.
        handshake.request({ finished++ }, presented = false)
        handshake.finish()
        handshake.request({ finished++ }, presented = true)
        assertEquals(listOf(2), reported); assertEquals(4, finished)
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
    @Test fun aNewGenerationWritesItsFirstFramesAndRequestsThenCounts() {
        val budget = CanvasLifeBudget(frames = 3, requests = 2, summaryMs = 1_000)
        assertEquals(listOf(true, true, true, false, false), (1..5).map(budget::frameWritten))
        assertEquals(listOf(true, true, false, false), List(4) { budget.request(granted = true) })
    }

    @Test fun hoverAtTwoHundredSamplesASecondWritesOneSummaryASecond() {
        val budget = CanvasLifeBudget(frames = 8, requests = 16, summaryMs = 1_000)
        repeat(16) { budget.request(granted = true) }
        repeat(8) { budget.frameWritten(it + 1) }
        val summaries = mutableListOf<String>()
        var frame = 8
        for (sample in 0 until 600) { // three seconds, one sample every 5 ms
            val now = 10_000L + sample * 5
            if (!budget.request(granted = sample % 2 == 0)) budget.summaryDue(now)?.let(summaries::add)
            if (sample % 2 == 0 && !budget.frameWritten(++frame)) { budget.quietFrame(); budget.summaryDue(now)?.let(summaries::add) }
        }
        assertEquals(3, summaries.size)
        assertEquals("quiet_requests=1 quiet_granted=1 quiet_frames=0 ink_frames=0", summaries.first())
        assertTrue(summaries[1], summaries[1].startsWith("quiet_requests=200 quiet_granted=100 quiet_frames=100 "))
    }

    @Test fun aLifecycleEventWritesTheNextFramesAgainAndHandsOverWhatWasCounted() {
        val budget = CanvasLifeBudget(frames = 2, requests = 1, summaryMs = 1_000)
        budget.request(granted = true)
        assertFalse(budget.request(granted = false)); budget.quietFrame(); budget.inkFrame(); budget.inkFrame()
        assertFalse(budget.frameWritten(3))
        assertEquals("quiet_requests=1 quiet_granted=0 quiet_frames=1 ink_frames=2", budget.again(started = 2, nowMs = 50))
        assertTrue(budget.frameWritten(3)); assertTrue(budget.frameWritten(4)); assertFalse(budget.frameWritten(5))
        assertTrue(budget.request(granted = true))
        assertNull("nothing new was counted", budget.again(started = 4, nowMs = 60))
        assertTrue("the event re-opened the requests", budget.request(granted = false))
    }

    /** A frame's start and completion are numbered in the same order, so a frame
     * whose start line was written also has its completion line written, even when
     * the next event arrives between them. */
    @Test fun aFrameWhoseStartWasWrittenHasItsCompletionWritten() {
        val budget = CanvasLifeBudget(frames = 1, requests = 0, summaryMs = 1_000)
        assertTrue(budget.frameWritten(1))
        budget.again(started = 1, nowMs = 0)
        assertTrue(budget.frameWritten(1)); assertTrue(budget.frameWritten(2))
    }

    @Test fun inkFramesAreCountedOnceWhereverTheyAreWritten() {
        val budget = CanvasLifeBudget(frames = 1, requests = 0, summaryMs = 1_000)
        repeat(5) { budget.inkFrame() }
        assertEquals(5, budget.takeInk()); assertEquals(0, budget.takeInk())
        budget.inkFrame()
        assertEquals("quiet_requests=0 quiet_granted=0 quiet_frames=0 ink_frames=1", budget.summaryDue(0))
        assertNull(budget.summaryDue(5_000))
    }

    @Test(expected = IllegalArgumentException::class) fun aSummaryGapMustBePositive() { CanvasLifeBudget(1, 1, 0) }
}
