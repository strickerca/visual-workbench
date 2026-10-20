package com.visualworkbench.android.editor

import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.junit.Assert.*
import org.junit.Test

/** Deterministic diagnostic ownership only; no Android/native/renderer proof. */
class EditorCloseProgressTest {
    @Test fun heldStageSettlesOnlyAfterItsActualCompletion() = runBlocking {
        withTimeout(2000) {
            val events = mutableListOf<EditorCloseSnapshot>()
            val progress = EditorCloseProgress(events::add)
            val attempt = progress.begin()
            val entered = CompletableDeferred<Unit>()
            val release = CompletableDeferred<Unit>()
            val work = launch(start = CoroutineStart.UNDISPATCHED) {
                try { attempt.step(EditorCloseStage.DocumentOwner) { entered.complete(Unit); release.await() } }
                finally { attempt.finish() }
            }
            try {
                entered.await()
                assertEquals(EditorCloseStage.DocumentOwner, progress.current?.stage)
                assertEquals(EditorCloseStatus.Running, progress.current?.status)
                assertFalse(work.isCompleted)
                assertFalse(events.any { it.stage == EditorCloseStage.DocumentOwner && it.status == EditorCloseStatus.Settled })
                release.complete(Unit)
                work.join()
                assertTrue(events.any { it.stage == EditorCloseStage.DocumentOwner && it.status == EditorCloseStatus.Settled })
                assertEquals(EditorCloseStatus.Completed, progress.current?.status)
            } finally { release.complete(Unit); withContext(NonCancellable) { work.join() } }
        }
    }

    @Test fun queuedNavigationCannotHideTheActiveBlockedJoinAndAdoptsItsOwnAttemptAfterward() = runBlocking {
        withTimeout(2000) {
            val events = mutableListOf<EditorCloseSnapshot>()
            val progress = EditorCloseProgress(events::add)
            val mutations = Mutex()
            val entered = CompletableDeferred<Unit>()
            val release = CompletableDeferred<Unit>()
            val first = progress.begin()
            val firstWork = launch(start = CoroutineStart.UNDISPATCHED) {
                try { first.step(EditorCloseStage.MutationGate) { mutations.withLock {
                    first.reached(EditorCloseStage.MutationAcquired)
                    first.step(EditorCloseStage.DocumentOwner) { entered.complete(Unit); release.await() }
                } } } finally { first.finish() }
            }
            var secondWork: Job? = null
            try {
                entered.await()
                val blocked = progress.current
                val second = progress.begin()
                val queuedWork = launch(start = CoroutineStart.UNDISPATCHED) {
                    try { second.step(EditorCloseStage.MutationGate) { mutations.withLock {
                        second.reached(EditorCloseStage.MutationAcquired)
                        second.step(EditorCloseStage.ProjectStorage) { }
                    } } } finally { second.finish() }
                }
                secondWork = queuedWork
                assertEquals(blocked, progress.current)
                val queued = events.single { it.stage == EditorCloseStage.Queued }
                assertNotEquals(blocked?.attempt, queued.attempt)
                assertFalse(queuedWork.isCompleted)
                release.complete(Unit)
                firstWork.join(); queuedWork.join()
                assertEquals(queued.attempt, progress.current?.attempt)
                assertEquals(EditorCloseStatus.Completed, progress.current?.status)
                val finishedFirst = events.indexOfFirst { it.attempt == blocked?.attempt && it.status == EditorCloseStatus.Completed }
                val acquiredSecond = events.indexOfFirst { it.attempt == queued.attempt && it.stage == EditorCloseStage.MutationAcquired }
                assertTrue(finishedFirst >= 0 && acquiredSecond > finishedFirst)
            } finally {
                release.complete(Unit)
                withContext(NonCancellable) { firstWork.join(); secondWork?.join() }
            }
        }
    }

    @Test fun finishingPostLockNavigationImmediatelyExposesAnAlreadySuspendedSuccessor() = runBlocking {
        withTimeout(2000) {
            val events = mutableListOf<EditorCloseSnapshot>()
            val progress = EditorCloseProgress(events::add)
            val mutations = Mutex()
            val firstEntered = CompletableDeferred<Unit>()
            val leaveFirstGate = CompletableDeferred<Unit>()
            val catalogEntered = CompletableDeferred<Unit>()
            val finishCatalog = CompletableDeferred<Unit>()
            val documentEntered = CompletableDeferred<Unit>()
            val finishDocument = CompletableDeferred<Unit>()
            var successorResumed = false
            val first = progress.begin()
            val firstWork = launch(start = CoroutineStart.UNDISPATCHED) {
                try {
                    first.step(EditorCloseStage.MutationGate) { mutations.withLock {
                        first.reached(EditorCloseStage.MutationAcquired)
                        firstEntered.complete(Unit); leaveFirstGate.await()
                    } }
                    first.step(EditorCloseStage.ProjectsCatalog) {
                        catalogEntered.complete(Unit); finishCatalog.await()
                    }
                } finally { first.finish() }
            }
            var secondWork: Job? = null
            try {
                firstEntered.await()
                val second = progress.begin()
                val queued = events.single { it.stage == EditorCloseStage.Queued }
                val queuedWork = launch(start = CoroutineStart.UNDISPATCHED) {
                    try { second.step(EditorCloseStage.MutationGate) { mutations.withLock {
                        second.reached(EditorCloseStage.MutationAcquired)
                        second.step(EditorCloseStage.DocumentOwner) {
                            documentEntered.complete(Unit); finishDocument.await(); successorResumed = true
                        }
                    } } } finally { second.finish() }
                }
                secondWork = queuedWork
                leaveFirstGate.complete(Unit)
                catalogEntered.await(); documentEntered.await()
                val blockedFirst = progress.current
                assertEquals(EditorCloseStage.ProjectsCatalog, blockedFirst?.stage)
                assertEquals(EditorCloseStatus.Running, blockedFirst?.status)
                assertNotEquals(queued.attempt, blockedFirst?.attempt)
                val beforeFinish = events.size
                finishCatalog.complete(Unit); firstWork.join()
                assertFalse(queuedWork.isCompleted)
                assertFalse(successorResumed)
                val expected = EditorCloseSnapshot(queued.attempt, EditorCloseStage.DocumentOwner, EditorCloseStatus.Running)
                assertEquals(expected, progress.current)
                assertEquals(expected, events.last())
                val handoff = events.drop(beforeFinish)
                val firstFinished = handoff.indexOfFirst {
                    it.attempt == blockedFirst?.attempt && it.status == EditorCloseStatus.Completed
                }
                assertTrue(firstFinished >= 0)
                assertTrue(handoff.indexOf(expected) > firstFinished)
                assertEquals(expected, handoff.last())
            } finally {
                leaveFirstGate.complete(Unit); finishCatalog.complete(Unit); finishDocument.complete(Unit)
                withContext(NonCancellable) { firstWork.join(); secondWork?.join() }
            }
        }
    }

    @Test fun finishedInactiveAttemptCannotReplaceTheActiveFailureDuringHandoff() = runBlocking {
        withTimeout(2000) {
            val events = mutableListOf<EditorCloseSnapshot>()
            val progress = EditorCloseProgress(events::add)
            val first = progress.begin()
            val failure = IllegalStateException("fixed active failure")
            val caught = runCatching { first.step(EditorCloseStage.Workflows) { throw failure } }.exceptionOrNull()
            assertSame(failure, caught)
            val activeFailure = progress.current
            val second = progress.begin()
            second.step(EditorCloseStage.MutationGate) { false }
            second.finish(completed = false)
            assertEquals(activeFailure, progress.current)
            first.finish()
            assertEquals(activeFailure, progress.current)
            assertEquals(EditorCloseStage.Workflows, progress.current?.failedStage)
            assertEquals(EditorCloseStatus.Failed, events.last().status)
            assertEquals(activeFailure?.attempt, events.last().attempt)
            val eventCount = events.size
            second.finish()
            assertEquals(eventCount, events.size)
            val third = progress.begin()
            third.step(EditorCloseStage.ProjectStorage) { }
            third.finish()
            assertEquals(EditorCloseStatus.Completed, progress.current?.status)
            assertNotEquals(activeFailure?.attempt, progress.current?.attempt)
        }
    }

    @Test fun nestedFailureKeepsItsExactStageAndThrowableThroughOuterWrappers() = runBlocking {
        withTimeout(2000) {
            val progress = EditorCloseProgress()
            val attempt = progress.begin()
            val failure = IllegalStateException("fixed synthetic failure")
            val caught = runCatching { attempt.step(EditorCloseStage.Navigation) {
                attempt.step(EditorCloseStage.MutationGate) {
                    attempt.step(EditorCloseStage.Workflows) { throw failure }
                }
            } }.exceptionOrNull()
            assertSame(failure, caught)
            assertEquals(EditorCloseStage.Workflows, progress.current?.stage)
            assertEquals(EditorCloseStage.Workflows, progress.current?.failedStage)
            attempt.finish()
            assertEquals(EditorCloseStatus.Failed, progress.current?.status)
            assertEquals(EditorCloseStage.Workflows, progress.current?.stage)
        }
    }

    @Test fun catalogueFailureAfterStorageCloseCannotBeReportedAsSuccessfulNavigation() = runBlocking {
        withTimeout(2000) {
            val events = mutableListOf<EditorCloseSnapshot>()
            val progress = EditorCloseProgress(events::add)
            val attempt = progress.begin()
            val failure = IllegalStateException("fixed catalogue failure")
            var storageClosed = false
            val caught = runCatching { attempt.step(EditorCloseStage.Navigation) {
                attempt.step(EditorCloseStage.ProjectStorage) { storageClosed = true }
                attempt.step(EditorCloseStage.ProjectsCatalog) { throw failure }
            } }.exceptionOrNull()
            assertSame(failure, caught)
            assertTrue(storageClosed)
            // Existing finally cleanup still runs after the navigation error.
            attempt.step(EditorCloseStage.NavigationInstructionCleanup) { }
            assertEquals(EditorCloseStage.ProjectsCatalog, progress.current?.failedStage)
            attempt.finish()
            assertEquals(EditorCloseStage.ProjectsCatalog, progress.current?.stage)
            assertEquals(EditorCloseStatus.Failed, progress.current?.status)
            assertFalse(events.any { it.status == EditorCloseStatus.Completed })
        }
    }

    @Test fun cancellationPropagatesTheExactExceptionAndRemainsFailure() = runBlocking {
        withTimeout(2000) {
            val progress = EditorCloseProgress()
            val attempt = progress.begin()
            val cancellation = CancellationException("fixed cancellation")
            val caught = runCatching { attempt.step(EditorCloseStage.DocumentOwner) { throw cancellation } }.exceptionOrNull()
            assertSame(cancellation, caught)
            attempt.finish()
            assertEquals(EditorCloseStage.DocumentOwner, progress.current?.failedStage)
            assertEquals(EditorCloseStatus.Failed, progress.current?.status)
        }
    }

    @Test fun cancelledOperationRemainsPendingUntilItsExistingRetirementFinallyJoins() = runBlocking {
        withTimeout(2000) {
            val progress = EditorCloseProgress()
            val attempt = progress.begin()
            val entered = CompletableDeferred<Unit>()
            val retiring = CompletableDeferred<Unit>()
            val release = CompletableDeferred<Unit>()
            val work = launch(start = CoroutineStart.UNDISPATCHED) {
                try { attempt.step(EditorCloseStage.DocumentOwner) {
                    try { entered.complete(Unit); awaitCancellation() }
                    finally { retiring.complete(Unit); withContext(NonCancellable) { release.await() } }
                } } finally { attempt.finish() }
            }
            try {
                entered.await(); work.cancel(); retiring.await()
                assertFalse(work.isCompleted)
                assertEquals(EditorCloseStatus.Running, progress.current?.status)
                release.complete(Unit); work.join()
                assertEquals(EditorCloseStatus.Failed, progress.current?.status)
                assertEquals(EditorCloseStage.DocumentOwner, progress.current?.failedStage)
            } finally { release.complete(Unit); withContext(NonCancellable) { work.join() } }
        }
    }

    @Test fun declinedNavigationIsDistinctFromCompletionAndFailure() = runBlocking {
        withTimeout(2000) {
            val progress = EditorCloseProgress()
            val attempt = progress.begin()
            attempt.step(EditorCloseStage.MutationGate) { false }
            attempt.finish(completed = false)
            assertEquals(EditorCloseStatus.Declined, progress.current?.status)
            assertNull(progress.current?.failedStage)
        }
    }

    @Test fun observerFailureCannotChangeTheOperationOrRetirementOutcome() = runBlocking {
        withTimeout(2000) {
            val progress = EditorCloseProgress { throw IllegalStateException("fixed observer failure") }
            val attempt = progress.begin()
            assertEquals(42, attempt.step(EditorCloseStage.ProjectStorage) { 42 })
            attempt.finish()
            assertEquals(EditorCloseStatus.Completed, progress.current?.status)
        }
    }
}
