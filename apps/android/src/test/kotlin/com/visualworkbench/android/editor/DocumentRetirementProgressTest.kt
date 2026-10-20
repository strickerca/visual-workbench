package com.visualworkbench.android.editor

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

/** Synthetic diagnostic ownership; no Android renderer or device execution. */
class DocumentRetirementProgressTest {
    @Test fun everyHeldJoinRemainsRunningUntilItsActualCompletion() = runBlocking {
        withTimeout(2000) {
            for (stage in listOf(DocumentRetirementStage.TaskJoin, DocumentRetirementStage.DrawGate,
                DocumentRetirementStage.Adopt, DocumentRetirementStage.SurfaceRetirement,
                DocumentRetirementStage.FrameRetirement, DocumentRetirementStage.CacheClose)) {
                val progress = DocumentRetirementProgress()
                val attempt = progress.begin()
                val entered = CompletableDeferred<Unit>()
                val release = CompletableDeferred<Unit>()
                val work = launch(start = CoroutineStart.UNDISPATCHED) {
                    try { attempt.step(stage) { entered.complete(Unit); release.await() } }
                    finally { attempt.finish() }
                }
                try {
                    entered.await()
                    assertEquals(DocumentRetirementSnapshot(1, stage, DocumentRetirementStatus.Running), progress.current)
                    assertFalse(work.isCompleted)
                    release.complete(Unit); work.join()
                    assertEquals(DocumentRetirementStatus.Completed, progress.current?.status)
                    assertNull(progress.current?.failedStage)
                } finally { release.complete(Unit); withContext(NonCancellable) { work.join() } }
            }
        }
    }

    @Test fun nestedFailurePreservesTheExactThrowableAndFirstFailedPhase() = runBlocking {
        withTimeout(2000) {
            val progress = DocumentRetirementProgress()
            val attempt = progress.begin()
            val failure = IllegalStateException("synthetic renderer failure")
            val caught = runCatching { attempt.step(DocumentRetirementStage.DrawGate) {
                attempt.reached(DocumentRetirementStage.DrawGateAcquired)
                attempt.step(DocumentRetirementStage.SurfaceRetirement) { throw failure }
            } }.exceptionOrNull()
            assertSame(failure, caught)
            attempt.finish()
            assertEquals(DocumentRetirementSnapshot(1, DocumentRetirementStage.SurfaceRetirement,
                DocumentRetirementStatus.Failed, DocumentRetirementStage.SurfaceRetirement), progress.current)
            val failed = progress.current
            attempt.finish()
            assertEquals(failed, progress.current)
        }
    }

    @Test fun cancellationRemainsRunningThroughTheExistingFinalJoinAndKeepsItsExactCause() = runBlocking {
        withTimeout(2000) {
            val progress = DocumentRetirementProgress()
            val attempt = progress.begin()
            val entered = CompletableDeferred<Unit>()
            val cleanup = CompletableDeferred<Unit>()
            val release = CompletableDeferred<Unit>()
            val cancellation = CancellationException("synthetic cancellation")
            var caught: Throwable? = null
            val work = launch(start = CoroutineStart.UNDISPATCHED) {
                try { attempt.step(DocumentRetirementStage.TaskJoin) {
                    try { entered.complete(Unit); awaitCancellation() }
                    finally { cleanup.complete(Unit); withContext(NonCancellable) { release.await() } }
                } } catch (error: Throwable) { caught = error; throw error }
                finally { attempt.finish() }
            }
            try {
                entered.await(); work.cancel(cancellation); cleanup.await()
                assertFalse(work.isCompleted)
                assertEquals(DocumentRetirementStatus.Running, progress.current?.status)
                release.complete(Unit); work.join()
                // Job cancellation can recover a coroutine stack trace. Its
                // cancellation object/cause must still be the supplied cause.
                assertTrue(caught === cancellation || caught?.cause === cancellation)
                assertEquals(DocumentRetirementStage.TaskJoin, progress.current?.failedStage)
                assertEquals(DocumentRetirementStatus.Failed, progress.current?.status)
            } finally { release.complete(Unit); withContext(NonCancellable) { work.join() } }
        }
    }

    @Test fun finishingAnAttemptImmediatelyPromotesItsAlreadySuspendedSuccessor() = runBlocking {
        withTimeout(2000) {
            val progress = DocumentRetirementProgress()
            val first = progress.begin()
            val second = progress.begin()
            val entered = CompletableDeferred<Unit>()
            val release = CompletableDeferred<Unit>()
            val work = launch(start = CoroutineStart.UNDISPATCHED) {
                try { second.step(DocumentRetirementStage.DrawGate) { entered.complete(Unit); release.await() } }
                finally { second.finish() }
            }
            try {
                entered.await()
                assertEquals(1, progress.current?.attempt)
                first.finish()
                assertFalse(work.isCompleted)
                assertEquals(DocumentRetirementSnapshot(2, DocumentRetirementStage.DrawGate,
                    DocumentRetirementStatus.Running), progress.current)
            } finally { first.finish(); release.complete(Unit); withContext(NonCancellable) { work.join() } }
        }
    }

    @Test fun anInactiveFinishedAttemptCannotHideAnActiveFailure() = runBlocking {
        withTimeout(2000) {
            val progress = DocumentRetirementProgress()
            val first = progress.begin()
            val failure = CancellationException("synthetic direct cancellation")
            assertSame(failure, runCatching { first.step(DocumentRetirementStage.TaskJoin) { throw failure } }.exceptionOrNull())
            val failed = progress.current
            val second = progress.begin()
            second.step(DocumentRetirementStage.CacheClose) { }
            second.finish(); first.finish()
            assertEquals(failed, progress.current)
        }
    }

    @Test fun wrappedDocumentAttemptNumbersDoNotReplaceTheActualActiveOwner() = runBlocking {
        withTimeout(2000) {
            val progress = DocumentRetirementProgress()
            val first = progress.begin()
            DocumentRetirementProgress::class.java.getDeclaredField("sequence").apply { isAccessible = true }
                .setInt(progress, Int.MAX_VALUE)
            val next = progress.begin()
            val original = progress.current
            next.reached(DocumentRetirementStage.DrawGateAcquired)
            assertEquals(original, progress.current)
            first.finish()
            assertEquals(DocumentRetirementSnapshot(1, DocumentRetirementStage.DrawGateAcquired,
                DocumentRetirementStatus.Settled), progress.current)
            first.reached(DocumentRetirementStage.CacheClose)
            assertEquals(DocumentRetirementStage.DrawGateAcquired, progress.current?.stage)
            next.finish()
        }
    }

    @Test fun libraryCallbackIsVisibleFromAnotherThreadBeforeMainDelivery() = runBlocking {
        withTimeout(2000) {
            val progress = CanvasRetirementProgress()
            val release = progress.begin(CanvasRetirementReason.ProjectRetirement)
            val mainDelivery = withContext(Dispatchers.Default) {
                release.reached(CanvasRetirementMilestone.LibraryCallbackEntered)
                val deliver: () -> Unit = { release.reached(CanvasRetirementMilestone.MainCallbackEntered) }
                deliver
            }
            assertEquals(CanvasRetirementMilestone.LibraryCallbackEntered, progress.current?.milestone)
            mainDelivery()
            assertEquals(CanvasRetirementMilestone.MainCallbackEntered, progress.current?.milestone)
        }
    }

    @Test fun staleReleaseCannotReplaceANewerGenerationEvenAfterTheCounterWraps() {
        val progress = CanvasRetirementProgress()
        val old = progress.begin(CanvasRetirementReason.SurfaceChanged)
        val oldNumber = progress.current?.generation
        CanvasRetirementProgress::class.java.getDeclaredField("sequence").apply { isAccessible = true }
            .setInt(progress, Int.MAX_VALUE)
        val current = progress.begin(CanvasRetirementReason.ProjectRetirement)
        assertEquals(oldNumber, progress.current?.generation)
        val expected = progress.current
        old.reached(CanvasRetirementMilestone.CompletionPublished)
        old.failed(CanvasRetirementMilestone.ReaderCloseRunning)
        assertEquals(expected, progress.current)
        current.reached(CanvasRetirementMilestone.LibraryCallbackEntered)
        assertEquals(CanvasRetirementReason.ProjectRetirement, progress.current?.reason)
        assertEquals(CanvasRetirementMilestone.LibraryCallbackEntered, progress.current?.milestone)
    }

    @Test fun readerFailureCannotBecomeCompletionOrLoseItsFirstFailedMilestone() {
        val progress = CanvasRetirementProgress()
        val release = progress.begin(CanvasRetirementReason.WindowDetached)
        release.reached(CanvasRetirementMilestone.LibraryCallbackEntered)
        release.reached(CanvasRetirementMilestone.MainCallbackEntered)
        release.reached(CanvasRetirementMilestone.ReaderCloseRunning)
        release.failed(CanvasRetirementMilestone.ReaderCloseRunning)
        val failed = progress.current
        release.reached(CanvasRetirementMilestone.CompletionPublished)
        release.failed(CanvasRetirementMilestone.ReleaseRequested)
        assertEquals(failed, progress.current)
        assertEquals(CanvasRetirementMilestone.Failed, progress.current?.milestone)
        assertEquals(CanvasRetirementMilestone.ReaderCloseRunning, progress.current?.failedMilestone)
    }

    @Test fun laterMilestonesCannotRegressWhenAnEarlierCallbackArrivesLate() {
        val progress = CanvasRetirementProgress()
        val release = progress.begin(CanvasRetirementReason.DocumentFrameChanged)
        release.reached(CanvasRetirementMilestone.ReaderClosed)
        release.reached(CanvasRetirementMilestone.LibraryCallbackEntered)
        assertEquals(CanvasRetirementMilestone.ReaderClosed, progress.current?.milestone)
        release.reached(CanvasRetirementMilestone.CompletionPublished)
        assertEquals(CanvasRetirementMilestone.CompletionPublished, progress.current?.milestone)
        assertNull(progress.current?.failedMilestone)
    }

    @Test fun timeoutSuffixUsesTheExactInitialSentinelsAndLegacyTail() {
        assertEquals(
            "doc_attempt=0 doc_stage=None doc_status=None doc_failed_stage=None " +
                "canvas_generation=0 canvas_reason=None canvas_milestone=None canvas_failed_milestone=None " +
                "close_attempt=0 close_stage=None close_status=None close_failed_stage=None",
            formatRetirementTimeoutSuffix(null, null, null),
        )
    }

    @Test fun timeoutSuffixSerializesTheHeldNestedJoinAndAlreadyStartedSurfaceRelease() = runBlocking {
        withTimeout(2000) {
            val doc = DocumentRetirementProgress()
            val canvas = CanvasRetirementProgress()
            val close = EditorCloseProgress()
            val docAttempt = doc.begin()
            val closeAttempt = close.begin()
            canvas.begin(CanvasRetirementReason.SurfaceChanged)
            val entered = CompletableDeferred<Unit>()
            val release = CompletableDeferred<Unit>()
            val work = launch(start = CoroutineStart.UNDISPATCHED) {
                try { closeAttempt.step(EditorCloseStage.DocumentOwner) {
                    docAttempt.step(DocumentRetirementStage.SurfaceRetirement) {
                        entered.complete(Unit); release.await()
                    }
                } } finally { docAttempt.finish(); closeAttempt.finish() }
            }
            try {
                entered.await()
                assertFalse(work.isCompleted)
                assertEquals(
                    "doc_attempt=1 doc_stage=SurfaceRetirement doc_status=Running doc_failed_stage=None " +
                        "canvas_generation=1 canvas_reason=SurfaceChanged canvas_milestone=ReleaseRequested canvas_failed_milestone=None " +
                        "close_attempt=1 close_stage=DocumentOwner close_status=Running close_failed_stage=None",
                    formatRetirementTimeoutSuffix(doc.current, canvas.current, close.current),
                )
            } finally { release.complete(Unit); withContext(NonCancellable) { work.join() } }
        }
    }

    @Test fun timeoutSuffixPreservesFirstFailuresAndMaximumCountersWithoutExceptionText() = runBlocking {
        withTimeout(2000) {
            val doc = DocumentRetirementProgress()
            val canvas = CanvasRetirementProgress()
            val close = EditorCloseProgress()
            for (progress in listOf(doc, canvas, close)) {
                progress.javaClass.getDeclaredField("sequence").apply { isAccessible = true }
                    .setInt(progress, Int.MAX_VALUE - 1)
            }
            val docAttempt = doc.begin()
            val closeAttempt = close.begin()
            val renderer = canvas.begin(CanvasRetirementReason.SurfaceDestroyed)
            renderer.failed(CanvasRetirementMilestone.ReaderCloseRunning)
            renderer.reached(CanvasRetirementMilestone.CompletionPublished)
            val failure = IllegalStateException("synthetic exception content must remain private")
            assertSame(failure, runCatching {
                closeAttempt.step(EditorCloseStage.DocumentOwner) {
                    docAttempt.step(DocumentRetirementStage.DrawGate) {
                        docAttempt.step(DocumentRetirementStage.FrameRetirement) { throw failure }
                    }
                }
            }.exceptionOrNull())
            docAttempt.finish(); closeAttempt.finish()
            val suffix = formatRetirementTimeoutSuffix(doc.current, canvas.current, close.current)
            val legacyTail = "close_attempt=2147483647 close_stage=DocumentOwner close_status=Failed close_failed_stage=DocumentOwner"
            assertEquals(
                "doc_attempt=2147483647 doc_stage=FrameRetirement doc_status=Failed doc_failed_stage=FrameRetirement " +
                    "canvas_generation=2147483647 canvas_reason=SurfaceDestroyed canvas_milestone=Failed canvas_failed_milestone=ReaderCloseRunning " +
                    legacyTail,
                suffix,
            )
            assertTrue(suffix.endsWith(legacyTail))
            assertFalse(suffix.contains(failure.message.orEmpty()))
        }
    }
}
