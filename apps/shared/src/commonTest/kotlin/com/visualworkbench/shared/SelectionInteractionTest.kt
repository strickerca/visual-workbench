package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine
import kotlin.test.*

class SelectionInteractionTest {
    @Test fun inactiveEditorDoesNotAcquireOrReadUnusedNativeSelectionCapability() {
        val f = SelectionFixture()
        f.capabilityFailure = SelectionFailure(SelectionFailureKind.Unsupported)
        repeat(10) { f.bind((it + 2).toULong()); f.ui.viewport(f.camera.copy(scale = 1.0 + it)) }
        assertEquals(0, f.capabilityCalls); assertEquals(0, f.api.documentCalls)
        assertFalse(f.ui.state.value.loading)
        // The unsupported injected project remains usable for ordinary editor
        // actions; selection failure is delivered only after its explicit use.
        f.ui.tool(SelectionTool.Rectangle)
        assertEquals(1, f.capabilityCalls); assertFalse(f.ui.state.value.canDraw)
        assertNotNull(f.ui.state.value.message); f.close()
    }
    @Test fun inactiveLateReadAndReboundProjectCannotReuseASelectionFacade() {
        val f = SelectionFixture(); val gate = CompletableDeferred<Unit>(); f.api.documentGate = gate
        f.ui.tool(SelectionTool.Rectangle)
        assertEquals(1, f.capabilityCalls); assertTrue(f.ui.state.value.loading)
        f.ui.active(false); gate.complete(Unit)
        assertNull(f.ui.state.value.document); assertFalse(f.ui.state.value.loading)
        immediate { f.ui.detach() }
        val other = SelectionFakeProject { f.info }
        val snapshot = DocumentSnapshot("document", "Fixture", 64u, 64u, 8u, emptyList(), RenderList(f.info, emptyList()))
        f.ui.bind(other, snapshot, 2L, 1u); f.ui.viewport(f.camera)
        assertEquals(1, f.capabilityCalls)
        f.ui.active(true)
        assertEquals(2, f.capabilityCalls); assertSame(other, f.capabilityRequests.last())
        assertNotNull(f.ui.state.value.document); f.close()
    }
    @Test fun rectangleUsesDownCameraAndCommitsExactlyOnce() {
        val f = SelectionFixture()
        f.ui.tool(SelectionTool.Rectangle)
        assertTrue(f.ui.begin(Point(10.0, 12.0), f.camera))
        f.ui.append(Point(20.0, 24.0)); assertTrue(f.ui.finish())
        assertFalse(f.ui.finish())
        val edit = f.api.edits.single()
        assertEquals(SelectionOperation.Rectangle(Rect(10.0, 12.0, 10.0, 12.0), SelectionCombine.Add), edit.operation)
        assertEquals("hash-a", edit.binding.stateHash)
        assertNotNull(f.ui.state.value.selection); assertEquals(1, f.refreshes)
        f.close()
    }
    @Test fun cancelledOrIncompleteLassoDoesNotWrite() {
        val f = SelectionFixture(); f.ui.tool(SelectionTool.Lasso)
        assertTrue(f.ui.begin(Point(1.0, 1.0), f.camera)); f.ui.append(Point(2.0, 2.0))
        assertFalse(f.ui.finish()); assertTrue(f.api.edits.isEmpty())
        f.ui.begin(Point(1.0, 1.0), f.camera); f.ui.append(Point(4.0, 4.0)); f.ui.cancelGesture()
        assertFalse(f.ui.finish()); assertTrue(f.api.edits.isEmpty()); f.close()
    }
    @Test fun subtractiveMaskEraserAndRefinementsUseCanonicalOperations() {
        val f = SelectionFixture(); f.select()
        f.ui.tool(SelectionTool.MaskEraser); f.ui.brush(7.0, 128u)
        assertTrue(f.ui.begin(Point(5.0, 6.0), f.camera)); f.ui.append(Point(7.0, 8.0)); f.ui.finish()
        assertEquals(SelectionOperation.Paint(listOf(Point(5.0, 6.0), Point(7.0, 8.0)), 7.0, 128u, SelectionCombine.Subtract), f.api.edits.last().operation)
        f.ui.refinementRadius(3u)
        SelectionRefinement.entries.forEach { assertTrue(f.ui.refine(it)) }
        assertEquals(listOf(SelectionOperation.Invert, SelectionOperation.Expand(3u), SelectionOperation.Shrink(3u), SelectionOperation.Feather(3u)), f.api.edits.takeLast(4).map { it.operation })
        f.close()
    }
    @Test fun intersectAndLassoRetainAllRealPointsWithoutPrediction() {
        val f = SelectionFixture(); f.select(); f.ui.tool(SelectionTool.Lasso); f.ui.combine(SelectionCombine.Intersect)
        f.ui.begin(Point(1.0, 1.0), f.camera); f.ui.append(Point(4.0, 1.0)); f.ui.append(Point(3.0, 7.0)); f.ui.finish()
        assertEquals(SelectionOperation.Lasso(listOf(Point(1.0, 1.0), Point(4.0, 1.0), Point(3.0, 7.0)), SelectionCombine.Intersect), f.api.edits.single().operation)
        f.close()
    }
    @Test fun boundedPathOverflowCancelsInsteadOfThinningOrSaving() {
        val f = SelectionFixture(); f.ui.tool(SelectionTool.Paint); f.ui.begin(Point(1.0, 1.0), f.camera)
        repeat(16_384) { f.ui.append(Point(it.toDouble() + 2, 1.0)) }
        assertNull(f.ui.state.value.draft); assertFalse(f.ui.finish()); assertTrue(f.api.edits.isEmpty()); f.close()
    }
    @Test fun delayedTilesCannotReappearAfterSameHostSequenceVisibleChange() {
        val f = SelectionFixture(); val gate = CompletableDeferred<Unit>(); f.api.tileGate = gate; f.select()
        assertNull(f.ui.state.value.preview)
        val next = f.info.copy(stateHash = "hash-pending")
        f.ui.observe(next, 2u); assertNull(f.ui.state.value.document)
        gate.complete(Unit); assertNull(f.ui.state.value.preview)
        f.api.tileGate = null; f.info = next; f.bind(2u)
        assertEquals("hash-pending", f.ui.state.value.preview?.binding?.stateHash)
        assertEquals(0uL, f.ui.state.value.document?.binding?.hostSeq)
        f.close()
    }
    @Test fun stalePickerIsRefusedBeforeProducerAndNewSelectionClearsIt() {
        val f = SelectionFixture(); f.select()
        val ticket = assertNotNull(f.ui.saveTicket(SelectionExportKind.Cutout, true))
        f.ui.newSelection(); assertFalse(f.ui.matches(ticket))
        immediate { assertFailsWith<SelectionFailure> { f.ui.export(ticket, "private", "private/out.png") } }
        assertEquals(0, f.api.exports); f.close()
    }
    @Test fun readRequestsCoalesceWhileOneNativeCallIsPending() {
        val f = SelectionFixture(); val gate = CompletableDeferred<Unit>(); f.api.tileGate = gate; f.select()
        val first = f.api.tileCalls
        repeat(30) { f.ui.viewport(f.camera.copy(center = Point(32.0 + it, 32.0))) }
        assertEquals(first, f.api.tileCalls)
        f.api.tileGate = null; gate.complete(Unit)
        assertEquals(first + 1, f.api.tileCalls); f.close()
    }
    @Test fun reactivatingRetainedMaskWaitsForSnapshotInsteadOfCreatingAnotherMask() {
        for (useTool in listOf(false, true)) {
            val f = SelectionFixture(); f.select(); f.ui.active(false)
            assertNull(f.ui.state.value.selection)
            val gate = CompletableDeferred<Unit>(); f.api.documentGate = gate
            if (useTool) f.ui.tool(SelectionTool.Rectangle) else f.ui.active(true)
            assertTrue(f.ui.state.value.loading); assertFalse(f.ui.state.value.canDraw)
            assertFalse(f.ui.begin(Point(1.0, 1.0), f.camera)); assertTrue(f.api.edits.isEmpty())
            gate.complete(Unit)
            assertTrue(f.ui.begin(Point(1.0, 1.0), f.camera)); assertTrue(f.ui.finish(Point(8.0, 8.0)))
            assertEquals("selection", (f.api.edits.single().target as SelectionTarget.Existing).selection.objectId)
            f.close()
        }
    }
    @Test fun detachWaitsForCancelledProducerBeforeProjectMayClose() {
        val f = SelectionFixture(); f.api.tileGate = CompletableDeferred(); f.api.settlement = CompletableDeferred(); f.select()
        var detached = false
        val close = f.scope.launch { f.ui.detach(); detached = true }
        assertFalse(detached); assertTrue(f.api.cancelled)
        f.api.settlement!!.complete(Unit); assertTrue(detached); assertTrue(close.isCompleted)
        assertNull(f.ui.state.value.preview); f.scope.cancel()
    }
    @Test fun failureRefreshesWithoutAutomaticGeometryRetry() {
        val f = SelectionFixture(); f.api.failure = SelectionFailure(SelectionFailureKind.Memory)
        f.ui.tool(SelectionTool.Rectangle); f.ui.begin(Point(1.0, 1.0), f.camera); f.ui.finish(Point(4.0, 4.0))
        assertEquals(1, f.api.edits.size); assertEquals(1, f.refreshes); assertFalse(f.ui.state.value.busy)
        assertTrue(f.ui.state.value.message?.contains("budget") == true); f.close()
    }
    @Test fun previewWindowIsBoundedExactPixelsIncludingDocumentEdges() {
        val core = SelectionFakeCore()
        val (tiles, limited) = selectionPreviewRegions(core, Camera(Point(0.0, 0.0), 1.0, 0.0, 20_000.0, 20_000.0), 20_000u, 20_000u)
        assertTrue(limited); assertEquals(16, tiles.size)
        assertEquals(1_048_576L, tiles.sumOf { it.width.toLong() * it.height.toLong() })
        val (edge, cropped) = selectionPreviewRegions(core, Camera(Point(0.0, 0.0), 1.0, 0.0, 257.0, 259.0), 257u, 259u)
        assertFalse(cropped); assertEquals(4, edge.size); assertEquals(SelectionRegion(256u, 256u, 1u, 3u), edge.last())
    }
}

private fun immediate(block: suspend () -> Unit) {
    var finished = false; var error: Throwable? = null
    block.startCoroutine(object : Continuation<Unit> {
        override val context = EmptyCoroutineContext
        override fun resumeWith(result: Result<Unit>) { finished = true; error = result.exceptionOrNull() }
    })
    assertTrue(finished, "Fixture unexpectedly suspended"); error?.let { throw it }
}
private class SelectionFixture {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    val camera = Camera(Point(32.0, 32.0), 1.0, 0.0, 64.0, 64.0)
    var info = ProjectInfo("project", "Fixture", "device", 1u, false, false, 0u, "hash-a", listOf("document"))
    val project = SelectionFakeProject { info }
    val api = SelectionFakeApi { info }
    var refreshes = 0
    var capabilityCalls = 0
    var capabilityFailure: Exception? = null
    val capabilityRequests = mutableListOf<WorkbenchProject>()
    val ui: SelectionInteraction = SelectionInteraction(SelectionFakeCore(), scope, { 1000L }, { receipt ->
        refreshes++; if (receipt != null) info = receipt.revision
        bind((refreshes + 1).toULong())
    }, { value -> capabilityCalls++; capabilityRequests += value; capabilityFailure?.let { throw it }; api })
    init { bind(1u); ui.viewport(camera) }
    fun bind(sequence: ULong) { ui.bind(project, DocumentSnapshot("document", "Fixture", 64u, 64u, 8u, emptyList(), RenderList(info, emptyList())), 1L, sequence) }
    fun select() { api.exists = true; ui.useObject("selection") }
    fun close() { immediate { ui.detach() }; scope.cancel() }
}
private class SelectionFakeApi(private val info: () -> ProjectInfo) : WorkbenchSelections {
    var exists = false
    var documentGate: CompletableDeferred<Unit>? = null
    var tileGate: CompletableDeferred<Unit>? = null
    var settlement: CompletableDeferred<Unit>? = null
    var cancelled = false
    var tileCalls = 0
    var documentCalls = 0
    var exports = 0
    var failure: SelectionFailure? = null
    val edits = mutableListOf<SelectionEdit>()
    private fun binding(revision: ProjectInfo = info()) = SelectionBinding("project", "document", "source", revision.hostSeq, revision.stateHash)
    override suspend fun document(documentId: String): SelectionDocument {
        documentCalls++; documentGate?.await()
        return SelectionDocument(binding(), 64u, 64u, 8u, info())
    }
    override suspend fun snapshot(binding: SelectionBinding, objectId: String, memoryBudgetBytes: ULong): SelectionSnapshot {
        if (!exists) throw SelectionFailure(SelectionFailureKind.Conflict)
        return SelectionSnapshot(binding, SelectionVersion(objectId, "mask", 1u), "layer", 64u, 64u, SelectionRegion(0u, 0u, 10u, 10u), true, true)
    }
    override suspend fun tiles(binding: SelectionBinding, selection: SelectionVersion, regions: List<SelectionRegion>, memoryBudgetBytes: ULong): SelectionTiles {
        tileCalls++
        try { tileGate?.await() }
        catch (error: CancellationException) { cancelled = true; withContext(NonCancellable) { settlement?.await() }; throw error }
        return SelectionTiles(binding, selection, regions.map { SelectionTile(it, ByteArray((it.width * it.height).toInt()) { 255.toByte() }) })
    }
    override suspend fun edit(options: SelectionEdit): SelectionReceipt {
        edits += options; failure?.let { throw it }; exists = true
        val revision = info().copy(stateHash = "hash-${edits.size}", nextLamport = info().nextLamport + 4u)
        val id = when (val target = options.target) { is SelectionTarget.New -> target.objectId; is SelectionTarget.Existing -> target.selection.objectId }
        return SelectionReceipt(options.transactionId, snapshot(binding(revision), id, 1u), revision)
    }
    override suspend fun exportFile(options: SelectionExportOptions): SelectionExportReceipt { exports++; error("Unexpected export producer") }
}
private class SelectionFakeCore : WorkbenchCore {
    private var next = 0
    override fun newId(unixMs: ULong) = "id-${next++}"
    override fun newDeviceId() = "device"
    override fun cameraMatrix(camera: Camera, inverse: Boolean) = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>) = points.toList()
    override suspend fun create(options: CreateProject): WorkbenchProject = error("Unused")
    override suspend fun open(path: String): WorkbenchProject = error("Unused")
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("Unused")
}
private class SelectionFakeProject(private val revision: () -> ProjectInfo) : WorkbenchProject {
    override val changes = emptyFlow<ProjectChange>()
    override suspend fun info() = revision()
    override suspend fun document(documentId: String): DocumentSnapshot = error("Unused")
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("Unused")
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("Unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Unused")
    override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("Unused")
    override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
    override suspend fun close() = Unit
}
