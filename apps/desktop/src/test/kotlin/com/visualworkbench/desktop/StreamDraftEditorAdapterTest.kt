package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Path

/** Exercises the real controller against borrowed in-memory projects. These
 * checks acquire no native project, bitmap, provider, window or stream module. */
class StreamDraftEditorAdapterTest {
    @Test fun retainPrecedesSuspendedAcquisitionAndReadyUsesTheExactExternalCamera() = streamDraftAdapterFixture { f ->
        val acquired = CompletableDeferred<Unit>()
        val release = f.gate()
        f.project.read = {
            assertNotNull(f.retained)
            assertFalse(checkNotNull(f.retained).ready)
            acquired.complete(Unit)
            release.await()
            f.project.snapshot()
        }
        val creating = async { f.create() }
        acquired.await()
        assertFalse(creating.isCompleted)
        assertNotNull(f.retained)
        assertEquals(0, f.project.closes)
        release.complete(Unit)
        val adapter = creating.await()
        assertSame(f.retained, adapter)
        assertTrue(adapter.ready)
        assertSame(adapter.controller.state, adapter.state)
        assertTrue(adapter.state.value.isStreamDraft)
        assertEquals(f.camera, adapter.state.value.view.camera)
        assertEquals("draft-document", adapter.state.value.document!!.documentId)
        assertNull(adapter.state.value.background)
        assertNull(adapter.state.value.documentFrame)
        assertFalse(adapter.state.value.giantActive)
        assertEquals(0, f.project.backgroundReads)
        assertEquals(0, f.core.opens)
        assertEquals(0, f.handoff.calls)
    }

    @Test fun initializationFailureReturnsTheRetainedChildAndBothReadinessAccessorsReportIt() = streamDraftAdapterFixture { f ->
        val failure = IllegalStateException("refused draft document")
        f.project.read = { throw failure }
        val adapter = f.create()
        assertSame(f.retained, adapter)
        assertSame(failure, runCatching { adapter.ready }.exceptionOrNull())
        assertSame(failure, runCatching { adapter.state }.exceptionOrNull())
        assertSame(failure, runCatching { adapter.sealEdits() }.exceptionOrNull())
        val child = f.childJob()
        assertTrue(child.isActive)
        adapter.close()
        assertTrue(child.isCompleted)
        assertEquals(1, f.retirements)
        assertEquals(0, f.project.closes)
        assertTrue(f.parentJob.isActive)
    }

    @Test fun cancelledFactoryDoesNotCancelOrDiscardItsLateBorrowedInitialization() = streamDraftAdapterFixture { f ->
        val acquired = CompletableDeferred<Unit>()
        val release = f.gate()
        f.project.read = {
            acquired.complete(Unit)
            try { release.await() }
            catch (error: CancellationException) { f.project.readCancellations++; throw error }
            f.project.snapshot()
        }
        val creating = launch { f.create() }
        acquired.await()
        val adapter = checkNotNull(f.retained)
        val child = f.childJob()
        creating.cancel()
        yield()
        assertFalse(creating.isCompleted)
        assertTrue(child.isActive)
        assertEquals(0, f.project.readCancellations)
        val closing = async(start = CoroutineStart.UNDISPATCHED) { adapter.close() }
        assertFalse(closing.isCompleted)
        assertEquals(0, f.retirements)
        release.complete(Unit)
        creating.join()
        closing.await()
        assertTrue(runCatching { adapter.ready }.exceptionOrNull() is CancellationException)
        assertTrue(child.isCompleted)
        assertEquals(1, f.retirements)
        assertEquals(0, f.project.closes)
    }

    @Test fun externalCloseWaitsForInitializationBeforeEnteringActualControllerDisposal() = streamDraftAdapterFixture { f ->
        val acquired = CompletableDeferred<Unit>()
        val releaseRead = f.gate()
        val disposalEntered = CompletableDeferred<Unit>()
        val releaseDisposal = f.gate()
        f.project.read = { acquired.complete(Unit); releaseRead.await(); f.project.snapshot() }
        val creating = async { f.create() }
        acquired.await()
        val adapter = checkNotNull(f.retained)
        val child = f.childJob()
        adapter.controller.setLocalTunnelRetirement { disposalEntered.complete(Unit); releaseDisposal.await() }
        val closing = async(start = CoroutineStart.UNDISPATCHED) { adapter.close() }
        assertFalse(disposalEntered.isCompleted)
        assertFalse(closing.isCompleted)
        releaseRead.complete(Unit)
        creating.await()
        disposalEntered.await()
        assertTrue(child.isActive)
        assertFalse(adapter.ready)
        assertEquals(0, f.retirements)
        releaseDisposal.complete(Unit)
        closing.await()
        assertTrue(child.isCompleted)
        assertEquals(1, f.retirements)
        assertEquals(0, f.project.closes)
    }

    @Test fun failedControllerDisposalKeepsTheScopeAndExactBorrowForExplicitRetry() = streamDraftAdapterFixture { f ->
        val adapter = f.create()
        val child = f.childJob()
        val failure = IllegalStateException("retained child cleanup")
        var fail = true
        var disposalAttempts = 0
        adapter.controller.setLocalTunnelRetirement {
            disposalAttempts++
            if (fail) throw failure
        }
        val thrown = checkNotNull(runCatching { adapter.close() }.exceptionOrNull())
        assertTrue(thrown is IllegalStateException)
        assertEquals(failure.message, thrown.message)
        assertTrue(generateSequence(thrown) { it.cause }.any { it === failure })
        assertSame(adapter, f.retained)
        assertTrue(child.isActive)
        assertFalse(adapter.ready)
        assertEquals(0, f.retirements)
        assertEquals(0, f.project.closes)
        assertTrue(runCatching { adapter.sealEdits() }.isFailure)
        fail = false
        adapter.close()
        assertTrue(child.isCompleted)
        assertNull(f.retained)
        assertEquals(1, f.retirements)
        assertEquals(2, disposalAttempts)
        adapter.close()
        assertEquals(1, f.retirements)
        assertEquals(2, disposalAttempts)
        assertEquals(0, f.project.closes)
    }

    @Test fun cancelledConcurrentClosersJoinOneDisposalBeforeRetiringTheScope() = streamDraftAdapterFixture { f ->
        val adapter = f.create()
        val child = f.childJob()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        var attempts = 0
        adapter.controller.setLocalTunnelRetirement { attempts++; entered.complete(Unit); release.await() }
        val first = launch(start = CoroutineStart.UNDISPATCHED) { adapter.close() }
        entered.await()
        val second = launch(start = CoroutineStart.UNDISPATCHED) { adapter.close() }
        first.cancel(); second.cancel()
        yield()
        assertFalse(first.isCompleted)
        assertFalse(second.isCompleted)
        assertTrue(child.isActive)
        assertEquals(0, f.retirements)
        release.complete(Unit)
        joinAll(first, second)
        assertEquals(1, attempts)
        assertEquals(1, f.retirements)
        assertTrue(child.isCompleted)
        assertEquals(0, f.project.closes)
    }

    @Test fun retirementCallbackRunsAfterJoinAndCanRetryWithoutDisposingTheControllerAgain() = streamDraftAdapterFixture { f ->
        var notificationAttempts = 0
        f.beforeRetired = {
            assertTrue(f.parentJob.children.none())
            if (++notificationAttempts == 1) error("retirement notification retained")
        }
        val adapter = f.create()
        val child = f.childJob()
        var disposals = 0
        adapter.controller.setLocalTunnelRetirement { disposals++ }
        assertTrue(runCatching { adapter.close() }.isFailure)
        assertTrue(child.isCompleted)
        assertSame(adapter, f.retained)
        assertFalse(adapter.ready)
        assertEquals(0, f.retirements)
        adapter.close()
        adapter.close()
        assertEquals(2, notificationAttempts)
        assertEquals(1, disposals)
        assertEquals(1, f.retirements)
        assertNull(f.retained)
        assertEquals(0, f.project.closes)
    }

    @Test fun sealDelegatesToTheRealAcceptedEditQueueAndBlocksLaterMutations() = streamDraftAdapterFixture { f ->
        val adapter = f.create()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.project.editGate = { entered.complete(Unit); release.await() }
        adapter.controller.create(Shape.Rectangle(Rect(1.0, 2.0, 3.0, 4.0)))
        entered.await()
        val sealing = async(start = CoroutineStart.UNDISPATCHED) { adapter.sealEdits() }
        assertFalse(sealing.isCompleted)
        adapter.controller.create(Shape.Ellipse(Rect(2.0, 3.0, 4.0, 5.0)))
        release.complete(Unit)
        sealing.await()
        assertEquals(1, f.project.edits)
        adapter.controller.create(Shape.Rectangle(Rect(3.0, 4.0, 5.0, 6.0)))
        yield()
        assertEquals(1, f.project.edits)
        assertEquals(f.camera, adapter.state.value.view.camera)
        assertNull(adapter.state.value.background)
        assertEquals(0, f.project.backgroundReads)
        assertEquals(0, f.project.closes)
    }

    @Test fun heldIndependentEraseRefusesSealAndLeavesLaterOrdinaryEditsAdmitted() = streamDraftAdapterFixture { f ->
        f.project.items = listOf(streamDraftAdapterObject())
        val adapter = f.create()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.project.editGate = { entered.complete(Unit); release.await() }
        val eraser = adapter.controller.erasers.interaction
        eraser.tool(EraserMode.Object)
        assertTrue(eraser.begin(Point(10.0, 10.0), f.camera))
        assertTrue(eraser.finish())
        entered.await()
        assertTrue(eraser.state.value.busy)
        assertFalse(adapter.state.value.busy) // This owner is independent of task's edit queue.
        val refusal = runCatching { adapter.sealEdits() }.exceptionOrNull()
        assertTrue(refusal is IllegalStateException)
        assertTrue(eraser.state.value.busy)
        assertEquals(0, f.project.committedEdits)
        assertSame(adapter, f.retained)
        release.complete(Unit)
        eraser.state.first { !it.busy }
        assertEquals(1, f.project.committedEdits)
        assertTrue(f.project.commands.first().single() is EditCommand.Delete)
        f.project.editGate = {}
        adapter.controller.create(Shape.Rectangle(Rect(1.0, 2.0, 3.0, 4.0)))
        adapter.state.first { !it.busy && f.project.committedEdits == 2 }
        assertEquals(2, f.project.edits)
        assertTrue(f.project.commands.last().single() is EditCommand.Create)
        adapter.sealEdits()
        assertFalse(eraser.state.value.ready)
        assertNull(adapter.state.value.background)
        assertEquals(0, f.project.closes)
    }

    @Test fun successfulSealFencesDirectSubcontrollerEntryPoints() = streamDraftAdapterFixture { f ->
        f.project.items = listOf(streamDraftAdapterObject())
        val adapter = f.create()
        val controller = adapter.controller
        controller.selectAll()
        assertEquals(setOf("draft-shape"), adapter.state.value.selected)
        adapter.sealEdits()
        val reads = f.project.infoReads
        val eraser = controller.erasers.interaction
        eraser.tool(EraserMode.Object)
        assertFalse(eraser.begin(Point(10.0, 10.0), f.camera))
        assertFalse(eraser.finish())
        assertFalse(eraser.state.value.ready)
        val selection = controller.selections.interaction
        selection.tool(SelectionTool.Rectangle)
        assertFalse(selection.begin(Point(10.0, 10.0), f.camera))
        assertFalse(selection.finish(Point(20.0, 20.0)))
        assertNull(selection.state.value.document)
        assertEquals(setOf("draft-shape"), adapter.state.value.selected)
        assertNull(controller.rotation.begin(Point(10.0, 10.0), Point(20.0, 10.0)))
        controller.instructionEditor.focusGlobal()
        assertFalse(controller.instructionEditor.state.value.busy)
        assertNull(controller.instructionEditor.state.value.field)
        controller.instructionEditor.place(Point(10.0, 10.0), ObjectStyle(0x000000ffu, 2.0))
        controller.instructionEditor.deleteMarker("draft-shape")
        assertFalse(controller.instructionEditor.state.value.busy)
        controller.semanticEditor.snapping(false)
        controller.semanticEditor.refresh()
        controller.semanticEditor.place(Point(10.0, 10.0), ObjectStyle(0x000000ffu, 2.0), Transform())
        assertFalse(controller.semanticEditor.state.value.busy)
        assertNull(controller.semanticEditor.state.value.binding)
        controller.create(Shape.Rectangle(Rect(1.0, 2.0, 3.0, 4.0)))
        yield()
        assertEquals(reads, f.project.infoReads)
        assertEquals(0, f.project.edits)
        assertEquals(0, f.project.closes)
    }

    @Test fun adapterPublishesPanAndZoomAfterTheExactInitialCamera() = streamDraftAdapterFixture { f ->
        val adapter = f.create()
        assertEquals(f.camera, adapter.state.value.view.camera)
        adapter.controller.pan(4.0, -6.0)
        val panned = f.camera.copy(center = Point(f.camera.center.x - 4.0, f.camera.center.y + 6.0))
        assertEquals(panned, adapter.state.value.view.camera)
        adapter.controller.zoom(2.0)
        assertEquals(panned.copy(scale = f.camera.scale * 2.0), adapter.state.value.view.camera)
        assertNull(adapter.state.value.background)
        assertEquals(0, f.project.backgroundReads)
    }

    @Test fun initializationCannotCloseAndJoinItsOwnChildScope() = streamDraftAdapterFixture { f ->
        f.project.read = { checkNotNull(f.retained).close(); f.project.snapshot() }
        val adapter = f.create()
        val failure = runCatching { adapter.ready }.exceptionOrNull()
        assertTrue(failure is IllegalStateException)
        assertTrue(failure!!.message!!.contains("outside the child scope"))
        adapter.close()
        assertEquals(1, f.retirements)
        assertEquals(0, f.project.closes)
    }

    @Test fun cancelledParentStillKeepsALateBorrowUntilExternalRetirementJoinsIt() = streamDraftAdapterFixture { f ->
        val acquired = CompletableDeferred<Unit>()
        val release = f.gate()
        f.project.read = {
            acquired.complete(Unit)
            try { release.await() }
            catch (_: CancellationException) {
                f.project.readCancellations++
                withContext(NonCancellable) { release.await() }
            }
            f.project.snapshot()
        }
        val creating = async { f.create() }
        acquired.await()
        val adapter = checkNotNull(f.retained)
        val child = f.childJob()
        f.parentJob.cancel()
        val closing = async(start = CoroutineStart.UNDISPATCHED) { adapter.close() }
        yield()
        assertFalse(closing.isCompleted)
        assertFalse(child.isCompleted)
        assertEquals(0, f.retirements)
        release.complete(Unit)
        creating.await()
        closing.await()
        assertEquals(1, f.project.readCancellations)
        assertTrue(runCatching { adapter.state }.exceptionOrNull() is CancellationException)
        assertTrue(child.isCompleted)
        assertEquals(1, f.retirements)
        assertEquals(0, f.project.closes)
    }
}

private fun streamDraftAdapterFixture(body: suspend CoroutineScope.(StreamDraftAdapterTestFixture) -> Unit) = runBlocking {
    withTimeout(5000) {
        val fixture = StreamDraftAdapterTestFixture(this)
        try { body(fixture) } finally {
            withContext(NonCancellable) {
                fixture.gates.forEach { it.complete(Unit) }
                fixture.beforeRetired = {}
                fixture.retained?.let { adapter ->
                    adapter.controller.setLocalTunnelRetirement {}
                    adapter.close()
                }
                fixture.parentJob.cancelAndJoin()
            }
        }
    }
}

private class StreamDraftAdapterTestFixture(parent: CoroutineScope) {
    val parentJob = SupervisorJob(parent.coroutineContext[Job])
    val parentScope = CoroutineScope(parent.coroutineContext + parentJob)
    val gates = mutableListOf<CompletableDeferred<Unit>>()
    val core = StreamDraftAdapterTestCore()
    val project = StreamDraftAdapterTestProject()
    val handoff = StreamDraftAdapterTestHandoff()
    val camera = Camera(Point(19.0, 29.0), 1.75, 0.23, 811.0, 613.0)
    var retained: StreamDraftEditorAdapter? = null
    var retirements = 0
    var beforeRetired: () -> Unit = {}
    fun gate() = CompletableDeferred<Unit>().also { gates += it }
    fun childJob(): Job = parentJob.children.single()
    suspend fun create() = StreamDraftEditorAdapter.create(parentScope = parentScope, core = core,
        handoff = handoff, location = LocationPolicy(Path.of("stream-draft-adapter-fixture").toAbsolutePath().normalize(), true, "memory-only fixture"),
        deviceId = "fixture-device", project = project, documentId = "draft-document", initialCamera = camera,
        retain = { adapter -> check(retained == null); retained = adapter },
        retired = { adapter ->
            beforeRetired()
            check(retained === adapter)
            retained = null
            retirements++
        })
}

private class StreamDraftAdapterTestProject : WorkbenchProject {
    var closes = 0
    var edits = 0
    var committedEdits = 0
    var infoReads = 0
    var items = emptyList<RenderItem>()
    val commands = mutableListOf<List<EditCommand>>()
    var backgroundReads = 0
    var readCancellations = 0
    private var revisionNumber = 1uL
    private fun revision() = ProjectInfo("draft-project", "Borrowed", "fixture-device", revisionNumber,
        true, false, revisionNumber, "revision-$revisionNumber", listOf("draft-document"))
    fun snapshot() = DocumentSnapshot("draft-document", "Borrowed", 120u, 90u, 8u,
        listOf(LayerInfo("draft-layer", "Annotations", true, false, 1.0, "normal")), RenderList(revision(), items))
    var read: suspend () -> DocumentSnapshot = { snapshot() }
    var editGate: suspend () -> Unit = {}
    override val changes = emptyFlow<ProjectChange>()
    override suspend fun info(): ProjectInfo { infoReads++; return revision() }
    override suspend fun document(documentId: String): DocumentSnapshot {
        check(documentId == "draft-document")
        return read()
    }
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage {
        backgroundReads++; error("The borrowed overlay must not read source pixels")
    }
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo {
        edits++; editGate()
        this.commands += commands
        val removed = commands.filterIsInstance<EditCommand.Delete>().map { it.objectId }.toSet()
        items = items.filterNot { it.objectId in removed }
        committedEdits++; revisionNumber++; return revision()
    }
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Unused")
    override suspend fun render(documentId: String, rectangle: Rect?) = snapshot().render
    override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
    override suspend fun close() { closes++ }
}

private class StreamDraftAdapterTestCore : WorkbenchCore {
    var opens = 0
    private var next = 0
    override fun newId(unixMs: ULong) = "draft-adapter-${next++}"
    override fun newDeviceId() = "fixture-device"
    override fun cameraMatrix(camera: Camera, inverse: Boolean) = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>) = points
    override suspend fun create(options: CreateProject): WorkbenchProject = error("Unused")
    override suspend fun open(path: String): WorkbenchProject { opens++; error("The draft borrows its supplied project") }
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("Unused")
}

private class StreamDraftAdapterTestHandoff : DesktopHandoff {
    var calls = 0
    override suspend fun paste(assumeSrgb: Boolean): ClipboardImage { calls++; error("Unused") }
    override suspend fun copy(path: Path, receipt: CompletedExport) { calls++; error("Unused") }
    override suspend fun stage(path: Path, receipt: CompletedExport): DesktopDragFile { calls++; error("Unused") }
}

@OptIn(ExperimentalUnsignedTypes::class)
private fun streamDraftAdapterObject() = RenderItem("draft-shape", "draft-layer", 1.0, "normal",
    Rect(3.0, 4.0, 20.0, 20.0), Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(),
    ObjectStyle(0x000000ffu, 2.0, fill = 0x000000ffu), Shape.Rectangle(Rect(3.0, 4.0, 20.0, 20.0)), false)
