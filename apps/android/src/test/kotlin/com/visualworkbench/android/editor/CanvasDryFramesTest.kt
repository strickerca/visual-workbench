package com.visualworkbench.android.editor

import org.junit.Assert.*
import org.junit.Test

/** Finite synthetic renderer schedules, not Android/GPU execution. */
class CanvasDryFramesTest {
    @Test fun recordingDoesNotReopenAdmissionWhileHardwareCompletionIsHeld() {
        val frames = CanvasDryFrames()
        val first = checkNotNull(frames.request())
        frames.drawStarted(listOf(first))
        repeat(10_000) { assertNull(frames.request()) }
        frames.drawCompleted()
        assertNotNull(frames.request())
    }

    @Test fun anAnonymousCompletionCannotAcknowledgeAnUndrawnApplicationRequest() {
        val frames = CanvasDryFrames()
        val request = checkNotNull(frames.request())
        frames.drawStarted(emptyList())
        frames.drawCompleted()
        assertNull(frames.request())
        frames.drawStarted(listOf(request))
        frames.drawCompleted()
        assertNotNull(frames.request())
    }

    @Test fun overlappingAutomaticAndApplicationDrawsDrainInEitherCompletionOrder() {
        for (automaticCompletesFirst in listOf(true, false)) {
            val frames = CanvasDryFrames()
            val request = checkNotNull(frames.request())
            // The constructor's anonymous draw can start before our queued draw.
            frames.drawStarted(emptyList())
            frames.drawStarted(listOf(request))
            val order = if (automaticCompletesFirst) listOf("automatic", "application")
                else listOf("application", "automatic")
            for ((index, _) in order.withIndex()) {
                frames.drawCompleted()
                if (index == 0) assertNull(frames.request())
            }
            assertNotNull(frames.request())
        }
    }

    @Test fun anAutomaticDrawAfterTheApplicationDrawAlsoKeepsAdmissionClosed() {
        val frames = CanvasDryFrames()
        val request = checkNotNull(frames.request())
        frames.drawStarted(listOf(request))
        frames.drawStarted(emptyList())
        frames.drawCompleted()
        assertNull(frames.request())
        frames.drawCompleted()
        assertNotNull(frames.request())
    }

    @Test fun anImplicitDrawAloneBlocksNewWorkUntilItsCompletion() {
        val frames = CanvasDryFrames()
        frames.drawStarted(emptyList())
        assertNull(frames.request())
        frames.drawCompleted()
        assertNotNull(frames.request())
    }

    @Test fun aStaleRequestIdentityCannotAcknowledgeItsSuccessor() {
        val frames = CanvasDryFrames()
        val first = checkNotNull(frames.request())
        frames.drawStarted(listOf(first)); frames.drawCompleted()
        val next = checkNotNull(frames.request())
        frames.drawStarted(listOf(first)); frames.drawCompleted()
        assertNull(frames.request())
        frames.drawStarted(listOf(next)); frames.drawCompleted()
        assertNotNull(frames.request())
    }

    @Test fun retiringSealsAdmissionDespiteLateDrawingAndCompletionCallbacks() {
        val frames = CanvasDryFrames()
        val request = checkNotNull(frames.request())
        frames.drawStarted(listOf(request))
        frames.close()
        frames.drawCompleted()
        frames.drawStarted(listOf(request))
        frames.drawCompleted()
        frames.close()
        assertNull(frames.request())
    }

    @Test fun manyPublicationsCoalesceToTheLatestSceneAfterActualCompletion() {
        val renderer = FiniteRenderer(reopenOnRecording = false)
        renderer.publish(1); renderer.drain()
        for (revision in 2..10_000) { renderer.publish(revision); renderer.drain() }
        assertEquals(listOf(1), renderer.recorded)
        assertEquals(1, renderer.hardware.size)
        renderer.completeHardware(); renderer.drain()
        assertEquals(listOf(1, 10_000), renderer.recorded)
        renderer.completeHardware(); renderer.drain()
        assertEquals(listOf(1, 10_000), renderer.recorded)
        assertFalse(renderer.stalled)
    }

    @Test fun theOldRecordingBoundaryCanStarveTheFinitePoolAndItsOwnCompletionQueue() {
        val old = FiniteRenderer(reopenOnRecording = true)
        val repaired = FiniteRenderer(reopenOnRecording = false)
        for (revision in 1..4) {
            old.publish(revision); old.drain()
            repaired.publish(revision); repaired.drain()
        }
        assertTrue(old.stalled)
        assertEquals(4, old.recorded.size)
        assertFalse(repaired.stalled)
        assertEquals(listOf(1), repaired.recorded)
        // Completions use the same executor as buffer acquisition. They cannot
        // run behind a blocked fourth acquisition in the old schedule.
        old.completeHardware(); old.drain()
        assertTrue(old.stalled)
        repaired.completeHardware(); repaired.drain()
        assertEquals(listOf(1, 4), repaired.recorded)
        assertFalse(repaired.stalled)
    }

    /** Models only the pinned API34 three-buffer/same-executor dependency.
     * A stalled acquisition stops this executor; GPU completions are held by
     * the test and queued on that same executor when released. */
    private class FiniteRenderer(private val reopenOnRecording: Boolean) {
        private val frames = CanvasDryFrames()
        private val executor = ArrayDeque<() -> Unit>()
        val hardware = mutableListOf<() -> Unit>()
        val recorded = mutableListOf<Int>()
        var stalled = false
            private set
        private var available = 3
        private var oldQueued = false
        private var latest = 0

        fun publish(revision: Int) { latest = revision; enqueueLatest() }

        private fun enqueueLatest() {
            val request = if (reopenOnRecording) {
                if (oldQueued) return
                oldQueued = true
                null
            } else frames.request() ?: return
            executor.addLast {
                if (reopenOnRecording) oldQueued = false
                else frames.drawStarted(listOf(checkNotNull(request)))
                val revision = latest
                recorded += revision
                executor.addLast {
                    if (available == 0) stalled = true
                    else {
                        available--
                        hardware += {
                            executor.addLast {
                                available++
                                if (!reopenOnRecording) frames.drawCompleted()
                                if (latest > revision) enqueueLatest()
                            }
                        }
                    }
                }
            }
        }

        fun completeHardware() {
            val callbacks = hardware.toList(); hardware.clear()
            callbacks.forEach { it() }
        }

        fun drain() {
            var steps = 0
            while (!stalled && executor.isNotEmpty()) {
                check(++steps <= 100)
                executor.removeFirst()()
            }
        }
    }
}
