package com.visualworkbench.desktop.capture

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.Flow
import org.junit.Assert.*
import org.junit.Test

class CaptureDeliveryTest {
    private class Project(private val describe: suspend () -> ProjectInfo = { error("unused") },
        private val release: suspend () -> Unit) : WorkbenchProject {
        override val changes: Flow<ProjectChange> get() = error("unused")
        override suspend fun info(): ProjectInfo = describe()
        override suspend fun document(documentId: String): DocumentSnapshot = error("unused")
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
        override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
        override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
        override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
        override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("unused")
        override suspend fun export(options: ExportOptions): ExportResult = error("unused")
        override suspend fun close() { release() }
    }

    @Test fun failedDestinationCloseRetainsTheConcreteProjectWithoutASecondClose() = runBlocking {
        val failure = CoreFailure(CoreFailureKind.Worker)
        var closes = 0
        val project = Project { closes++; throw failure }
        var held: WorkbenchProject? = null
        var pending: Exception? = null
        val result = runCatching {
            deliverCaptureProject(project, { original ->
                assertSame(project, original) // Native delivery needs its actual concrete instance.
                try { /* destination delivery */ }
                finally { withContext(NonCancellable) { original.close() } }
            }) { original, error -> held = original; pending = error }
        }
        assertSame(project, held); assertSame(failure, pending)
        assertSame(failure, result.exceptionOrNull()); assertEquals(1, closes)
    }

    @Test fun cancellationAfterAffirmativeDestinationCloseKeepsCancellationSemantics() = runBlocking {
        var closes = 0
        val project = Project { closes++ }
        val cancellation = CancellationException("destination retired")
        var held = false
        val result = runCatching {
            deliverCaptureProject(project, { original ->
                try { throw cancellation }
                finally { withContext(NonCancellable) { original.close() } }
            }) { _, _ -> held = true }
        }
        assertSame(cancellation, result.exceptionOrNull()); assertFalse(held); assertEquals(1, closes)
    }

    @Test fun nativeCloseFailureReplacesDeliveryCancellationAndIsRetained() = runBlocking {
        val failure = CoreFailure(CoreFailureKind.Cancelled)
        val project = Project { throw failure }
        var pending: Exception? = null
        val result = runCatching {
            deliverCaptureProject(project, { original ->
                try { throw CancellationException("destination retired") }
                finally { withContext(NonCancellable) { original.close() } }
            }) { original, error -> assertSame(project, original); pending = error }
        }
        assertSame(failure, pending); assertSame(failure, result.exceptionOrNull())
    }

    private fun revision(): ProjectInfo = ProjectInfo("project", "Synthetic capture", "device", 3uL,
        true, false, 2uL, "a".repeat(64), listOf("document"))

    @Test fun observerUsesFinalMetadataReadBeforeDestinationCloseAndRunsAfterDelivery() = runBlocking {
        val expected = revision(); val order = mutableListOf<String>(); var closes = 0
        val project = Project(describe = { order += "info"; expected }) { order += "close"; closes++ }
        deliverCaptureProject(project, { it.close(); order += "delivered" }, observation = { actual ->
            assertSame(expected, actual); assertEquals(1, closes); order += "observed"
        }) { _, _ -> fail("Successful delivery must not retain a closed project") }
        assertEquals(listOf("info", "close", "delivered", "observed"), order)
    }

    @Test fun observerFailureReportsUnavailableWithoutRetainingOrClosingDeliveredProjectAgain() = runBlocking {
        for (failure in listOf(IllegalStateException("synthetic origin refusal"), CancellationException("service retired"))) {
            var closes = 0; var unavailable = 0
            val project = Project(describe = { revision() }) { closes++ }
            deliverCaptureProject(project, { it.close() }, observation = { throw failure },
                observationUnavailable = { unavailable++ }) { _, _ -> fail("Observer cannot retain successful destination ownership") }
            assertEquals(1, closes); assertEquals(1, unavailable)
        }
    }

    @Test fun optionalMetadataFailureStillDeliversAndClosesTheOriginalExactlyOnce() = runBlocking {
        var closes = 0; var unavailable = 0; var deliveries = 0
        val project = Project(describe = { throw CoreFailure(CoreFailureKind.Worker) }) { closes++ }
        deliverCaptureProject(project, { deliveries++; it.close() }, observation = { fail("No metadata was obtained") },
            observationUnavailable = { unavailable++ }) { _, _ -> fail("Optional metadata cannot fail destination delivery") }
        assertEquals(1, deliveries); assertEquals(1, closes); assertEquals(1, unavailable)
    }

    @Test fun destinationFailureRetainsItsExactOwnerAndNeverRecordsAnOrigin() = runBlocking {
        val error = CoreFailure(CoreFailureKind.Worker); var closes = 0; var observed = false
        val project = Project(describe = { revision() }) { closes++; throw error }
        var held: WorkbenchProject? = null
        val result = runCatching {
            deliverCaptureProject(project, { it.close() }, observation = { observed = true }) { original, failure ->
                held = original; assertSame(error, failure)
            }
        }
        assertSame(error, result.exceptionOrNull()); assertSame(project, held)
        assertFalse(observed); assertEquals(1, closes)
    }

    @Test fun cancellationAfterDeliveryJoinsOriginObservationWithoutAnotherProjectClose() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val settle = CompletableDeferred<Unit>()
        var closes = 0; var observed = false
        val project = Project(describe = { revision() }) { closes++ }
        val task = launch(start = CoroutineStart.UNDISPATCHED) {
            deliverCaptureProject(project, { it.close() }, observation = {
                entered.complete(Unit); settle.await(); observed = true
            }) { _, _ -> fail("Joined observation cannot acquire destination ownership") }
        }
        try {
            entered.await(); task.cancel()
            assertFalse(task.isCompleted); assertEquals(1, closes); assertFalse(observed)
            settle.complete(Unit); withTimeout(2000) { task.join() }
            assertTrue(observed); assertEquals(1, closes)
        } finally { settle.complete(Unit); withContext(NonCancellable) { task.cancelAndJoin() } }
    }
}
