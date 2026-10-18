package com.visualworkbench.desktop.capture

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.Flow
import org.junit.Assert.*
import org.junit.Test

class CaptureDeliveryTest {
    private class Project(private val release: suspend () -> Unit) : WorkbenchProject {
        override val changes: Flow<ProjectChange> get() = error("unused")
        override suspend fun info(): ProjectInfo = error("unused")
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
}
