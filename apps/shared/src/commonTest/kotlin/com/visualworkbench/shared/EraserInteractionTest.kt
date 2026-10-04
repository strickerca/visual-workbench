@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine
import kotlin.test.*

/** Only the three eraser ports are injected; no second fake WorkbenchCore or
 * geometry engine. Platform tests below exercise real painted-path hit logic. */
class EraserInteractionTest {
    @Test fun downCameraRealSamplesAndOneImmutableStrokeRequest() {
        val f = EraseFixture(); f.ui.tool(EraserMode.Stroke)
        assertTrue(f.ui.begin(Point(20.0, 24.0), f.camera))
        f.ui.append(Point(24.0, 28.0)); assertTrue(f.ui.finish(Point(28.0, 32.0)))
        assertFalse(f.ui.finish())
        val request = f.port.strokes.single()
        assertEquals(listOf(Point(10.0, 12.0), Point(12.0, 14.0), Point(14.0, 16.0)), request.centers)
        assertEquals("visible-a", request.binding.stateHash)
        assertEquals(1, f.refreshes); assertFalse(f.ui.state.value.busy); assertNull(f.ui.state.value.draft)
        assertEquals(f.id, f.lastChange?.replacements?.keys?.single()); f.close()
    }
    @Test fun cancellingDownDoesNotCallEitherMutation() {
        val f = EraseFixture(); f.ui.tool(EraserMode.Stroke); f.ui.begin(Point(10.0, 10.0), f.camera)
        f.ui.append(Point(20.0, 20.0)); f.ui.cancelGesture()
        assertFalse(f.ui.finish()); assertTrue(f.port.strokes.isEmpty()); assertTrue(f.port.deletes.isEmpty()); f.close()
    }
    @Test fun incomingVisibleChangeInvalidatesBeforeAsyncEditorRefresh() {
        val f = EraseFixture(); f.ui.tool(EraserMode.Stroke); f.ui.begin(Point(10.0, 10.0), f.camera)
        f.ui.observe(f.info.copy(stateHash = "pending-b"), 2u)
        assertNull(f.ui.state.value.draft); assertFalse(f.ui.state.value.ready); assertFalse(f.ui.finish())
        f.bind(2u) // Android publish can still carry the old snapshot while refresh is pending.
        assertFalse(f.ui.state.value.ready); assertFalse(f.ui.begin(Point(10.0, 10.0), f.camera))
        f.info = f.info.copy(stateHash = "pending-b"); f.bind(2u)
        assertTrue(f.ui.begin(Point(10.0, 10.0), f.camera)); f.ui.finish()
        assertEquals(0uL, f.port.strokes.single().binding.hostSeq)
        assertEquals("pending-b", f.port.strokes.single().binding.stateHash); f.close()
    }
    @Test fun sameSequenceNewAttachmentAndDocumentFenceCannotReuseGesture() {
        val f = EraseFixture(); f.ui.tool(EraserMode.Stroke); f.ui.begin(Point(10.0, 10.0), f.camera)
        f.epoch = 2; f.bind(1u); assertFalse(f.ui.finish())
        f.ui.begin(Point(10.0, 10.0), f.camera)
        f.documentId = "different-document"; f.bind(1u); assertFalse(f.ui.finish())
        assertTrue(f.port.strokes.isEmpty()); f.close()
    }
    @Test fun staleObjectHitCompletionCannotDeleteAtAnotherVisibleRevision() {
        val f = EraseFixture(); val gate = CompletableDeferred<Unit>(); f.port.hitGate = gate
        f.ui.tool(EraserMode.Object); f.ui.begin(Point(10.0, 10.0), f.camera); f.ui.finish()
        assertTrue(f.ui.state.value.busy)
        f.info = f.info.copy(stateHash = "remote-b"); f.ui.observe(f.info, 2u); f.bind(2u)
        gate.complete(Unit)
        assertTrue(f.port.deletes.isEmpty()); assertFalse(f.ui.state.value.busy)
        assertTrue(f.ui.state.value.message!!.contains("changed")); f.close()
    }
    @Test fun viewportChangeDuringHitWorkRefusesPublication() {
        val f = EraseFixture(); val gate = CompletableDeferred<Unit>(); f.port.hitGate = gate
        f.ui.tool(EraserMode.Object); f.ui.begin(Point(10.0, 10.0), f.camera); f.ui.finish()
        f.ui.viewport(f.camera.copy(scale = 3.0)); gate.complete(Unit)
        assertTrue(f.port.deletes.isEmpty()); f.close()
    }
    @Test fun detachSettlesWorkerAndRejectsProjectReplacementUntilThen() {
        val f = EraseFixture(); f.port.hitGate = CompletableDeferred(); f.port.settlement = CompletableDeferred()
        f.ui.tool(EraserMode.Object); f.ui.begin(Point(10.0, 10.0), f.camera); f.ui.finish()
        var closed = false
        val job = f.scope.launch { f.ui.detach(); closed = true }
        assertTrue(f.port.wasCancelled); assertFalse(closed)
        val writes = f.port.deletes.size
        f.ui.bind(ErasePort(f), f.snapshot(), 2, 1u) // ignored during detach
        assertEquals(writes, f.port.deletes.size)
        f.port.settlement!!.complete(Unit)
        assertTrue(closed); assertTrue(job.isCompleted); assertEquals(0, f.refreshes)
        assertFalse(f.ui.state.value.ready); f.scope.cancel()
    }
    @Test fun objectCommitCarriesFullCasAndExactlyTheHitIds() {
        val f = EraseFixture(); f.ui.tool(EraserMode.Object)
        f.ui.begin(Point(10.0, 10.0), f.camera); f.ui.finish()
        val (options, ids) = f.port.deletes.single()
        assertEquals(listOf(f.id), ids); assertEquals("visible-a", options.expectedStateHash)
        assertEquals(0uL, options.expectedHostSeq); assertEquals(f.info.deviceId, options.deviceId)
        assertEquals(1, f.refreshes); f.close()
    }
    @Test fun nativeFailureRefreshesOnceWithoutFreshIdRetry() {
        val f = EraseFixture(); f.port.failure = WorkflowFailure(WorkflowFailureKind.Memory)
        f.ui.tool(EraserMode.Stroke); f.ui.begin(Point(10.0, 10.0), f.camera); f.ui.finish()
        assertEquals(1, f.port.strokes.size); assertEquals(1, f.refreshes)
        assertTrue(f.ui.state.value.message!!.contains("budget")); assertFalse(f.ui.state.value.busy)
        f.port.failure = null; f.ui.begin(Point(10.0, 10.0), f.camera); f.ui.finish()
        assertEquals(2, f.port.strokes.size)
        assertNotEquals(f.port.strokes[0].metadata.transactionId, f.port.strokes[1].metadata.transactionId); f.close()
    }
    @Test fun acceptedReceiptSurvivesCancellationAtRefreshBoundary() {
        val f = EraseFixture(); f.cancelOnRefresh = true; val gate = CompletableDeferred<Unit>(); f.port.eraseGate = gate
        f.ui.tool(EraserMode.Stroke); f.ui.begin(Point(10.0, 10.0), f.camera); f.ui.finish()
        assertTrue(f.ui.state.value.busy); gate.complete(Unit)
        assertEquals(1, f.refreshes); assertNotNull(f.lastChange)
        assertEquals(1, f.port.strokes.size); assertFalse(f.ui.state.value.busy); f.close()
    }
    @Test fun oversizedRealPathCancelsWithoutDecimationOrWrite() {
        val f = EraseFixture(); f.ui.tool(EraserMode.Stroke); f.ui.begin(Point(0.0, 0.0), f.camera)
        repeat(4_096) { f.ui.append(Point(it + 1.0, 0.0)) }
        assertNull(f.ui.state.value.draft); assertFalse(f.ui.finish()); assertTrue(f.port.strokes.isEmpty()); f.close()
    }
    @Test fun noHitsAndTooManyTargetsNeverCallNativeMutation() {
        val f = EraseFixture(); f.ui.tool(EraserMode.Stroke)
        f.ui.begin(Point(1_000.0, 1_000.0), f.camera); f.ui.finish(); assertTrue(f.port.strokes.isEmpty())
        f.items = List(33) { f.item.copy(objectId = "target-$it") }
        // The no-hit completion already published a refresh. Admit this new
        // document at a newer revision/event rather than replaying that event.
        f.info = f.info.copy(hostSeq = 1uL, stateHash = "visible-many", nextLamport = 2uL)
        f.bind(f.seq + 1uL)
        f.ui.begin(Point(10.0, 10.0), f.camera); f.ui.finish(); assertTrue(f.port.strokes.isEmpty())
        assertTrue(f.ui.state.value.message!!.contains("budget")); f.close()
    }
    @Test fun platformCannotInjectLockedOrUnrelatedDeleteTargets() {
        val f = EraseFixture(); f.items = listOf(f.item.copy(locked = true)); f.bind(2u)
        f.ui.tool(EraserMode.Object); f.ui.begin(Point(10.0, 10.0), f.camera); f.ui.finish()
        assertTrue(f.port.deletes.isEmpty()); f.close()
    }
    @Test fun selectedMaskLoadingBlocksBothNormalAndHardwareContacts() {
        val vector = EraserUiState(active = true, mode = EraserMode.Object, ready = true)
        val loading = SelectionUiState(active = true, tool = SelectionTool.MaskEraser, loading = true)
        assertEquals(EraserInputRoute.Blocked, eraserInputRoute(loading, vector))
        assertEquals(EraserInputRoute.Blocked, eraserInputRoute(loading, vector, true))
        assertEquals(EraserInputRoute.Blocked, eraserInputRoute(loading.copy(loading = false), vector))
        assertEquals(EraserInputRoute.Eraser, eraserInputRoute(SelectionUiState(), vector.copy(active = false), true))
        assertEquals(EraserInputRoute.Editor, eraserInputRoute(SelectionUiState(), vector.copy(active = false)))
    }
    @Test fun hitAdmissionCountsHeadersAndRefusesWorkBeforePlatformCalls() {
        val f = EraseFixture(); val many = f.snapshot().copy(render = RenderList(f.info, List(513) { f.item }))
        assertFailsWith<WorkflowFailure> { admitEraserHit(EraserHitQuery(many, f.camera, listOf(Point(1.0, 1.0)), 12.0)) }
        val text = f.item.copy(shape = Shape.Text(Point(0.0, 0.0), "x", "Inter", 12.0, List(4_097) { Outline.Close }))
        assertFailsWith<WorkflowFailure> { admitEraserHit(EraserHitQuery(f.snapshot().copy(render = RenderList(f.info, listOf(text))), f.camera, listOf(Point(1.0, 1.0)), 12.0)) }
        f.close()
    }
}

private class EraseFixture {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    val camera = Camera(Point(32.0, 32.0), 2.0, 0.0, 128.0, 128.0)
    var info = ProjectInfo("project", "fixture", "device", 1u, false, false, 0u, "visible-a", listOf("document"))
    var documentId = "document"; var epoch = 1L; var seq = 1uL; var counter = 0
    val id = "00000000-0000-7000-8000-000000000010"
    val item = RenderItem(id, "layer", 1.0, "normal", Rect(0.0, 0.0, 40.0, 40.0),
        Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(), ObjectStyle(0xff0000ffu, 2.0), Shape.Stroke("pen"), false)
    var items = listOf(item)
    var refreshes = 0; var lastChange: EraserChange? = null; var cancelOnRefresh = false
    val port = ErasePort(this)
    val ui = EraserInteraction(scope, { view, _, points -> points.map { Point(it.x / view.scale, it.y / view.scale) } },
        { "00000000-0000-7000-8000-${(++counter).toString().padStart(12, '0')}" }, { 1_000L }) { change ->
        refreshes++; lastChange = change
        if (cancelOnRefresh) uiCancel()
        if (change != null) info = change.revision
        bind(++seq)
    }
    init { bind(seq) }
    private fun uiCancel() { ui.cancelOperation() }
    fun snapshot() = DocumentSnapshot(documentId, "fixture", 64u, 64u, 8u,
        listOf(LayerInfo("layer", "layer", true, false, 1.0, "normal")), RenderList(info, items))
    fun bind(sequence: ULong) { seq = sequence; ui.bind(port, snapshot(), epoch, sequence); ui.viewport(camera) }
    fun close() { eraseImmediate { ui.detach() }; scope.cancel() }
}
private class ErasePort(private val f: EraseFixture) : EraserBackend {
    val strokes = mutableListOf<VectorEraseEdit>()
    val deletes = mutableListOf<Pair<EditOptions, List<String>>>()
    var hitGate: CompletableDeferred<Unit>? = null
    var eraseGate: CompletableDeferred<Unit>? = null
    var settlement: CompletableDeferred<Unit>? = null
    var wasCancelled = false
    var failure: Exception? = null
    override suspend fun objectHits(query: EraserHitQuery): List<String> {
        try { hitGate?.await() }
        finally { if (!currentCoroutineContext().isActive) { wasCancelled = true; withContext(NonCancellable) { settlement?.await() } } }
        return listOf(f.id)
    }
    override suspend fun eraseStrokes(request: VectorEraseEdit): VectorEraseReceipt {
        strokes += request; eraseGate?.await(); failure?.let { throw it }
        return VectorEraseReceipt(request.metadata.transactionId,
            request.targets.map { VectorEraseReplacement(it.objectId, it.replacementId, 2u) }, f.info.copy(stateHash = "saved-${strokes.size}", nextLamport = f.info.nextLamport + 2u), 1u)
    }
    override suspend fun deleteObjects(options: EditOptions, objectIds: List<String>): ProjectInfo {
        deletes += options to objectIds; failure?.let { throw it }
        return f.info.copy(stateHash = "deleted", nextLamport = f.info.nextLamport + 1u)
    }
}
private fun eraseImmediate(block: suspend () -> Unit) {
    var completed: Result<Unit>? = null
    block.startCoroutine(object : Continuation<Unit> { override val context = EmptyCoroutineContext
        override fun resumeWith(result: Result<Unit>) { completed = result } })
    checkNotNull(completed) { "Unexpected suspended fixture cleanup" }.getOrThrow()
}
