@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import android.app.Application
import android.view.MotionEvent
import com.visualworkbench.android.input.CanvasGestureRouter
import com.visualworkbench.android.input.PointerFrame
import com.visualworkbench.android.input.PointerPacket
import androidx.lifecycle.ViewModelStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.android.editor.WorkbenchScreen
import com.visualworkbench.android.editor.EditorTool
import com.visualworkbench.android.input.PenSample
import com.visualworkbench.android.input.PointerTool
import com.visualworkbench.android.input.StylusCapabilities
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Actual editor/link/refresh owners with synthetic native DTOs. No device capture or network proof. */
@RunWith(AndroidJUnit4::class)
class CaptureDeliveryInstrumentedTest {
    @Test fun verifiedIncomingCaptureKeepsCurrentLinkAndDefersWhileBrushDialogIsOpen(): Unit = runBlocking {
        var incoming: ReceivedCapture? = null
        fixture(query = { _, after -> incoming?.takeIf { it.createdHostSeq > after } }) { editor, core, link ->
            connect(editor)
            val oldDoc = core.current.documentId
            withContext(Dispatchers.Main) {
                val info = core.current.info()
                incoming = ReceivedCapture(WorkflowBinding(info.projectId, "received-capture", 1uL, "state-1"), 1uL,
                    "a".repeat(64), "synthetic-session", 3uL, 7u, true)
                editor.brushDialog(true); core.current.change(1uL, 0xff00ffffu)
            }
            waitUntil { editor.captureArrivals.state.value != null }
            withContext(Dispatchers.Main) {
                editor.openReceivedCapture(checkNotNull(incoming)); assertEquals(oldDoc, editor.document?.documentId)
                assertFalse(editor.canOpenReceivedCapture()); editor.brushDialog(false)
            }
            waitUntil { editor.canOpenReceivedCapture() }
            withContext(Dispatchers.Main) { editor.openReceivedCapture(checkNotNull(incoming)) }
            waitUntil { !editor.busy && editor.document?.documentId == "received-capture" }
            withContext(Dispatchers.Main) {
                assertNull(editor.captureArrivals.state.value); assertFalse(core.current.closed)
                assertFalse(link.events.contains("link-close")); assertNotNull(editor.connection)
                assertNotNull(editor.scene?.background)
            }
        }
    }
    @Test fun modalOpenedDuringDisplayPreparationPreservesPriorDocumentAndPendingCapture(): Unit = runBlocking {
        var incoming: ReceivedCapture? = null
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            fixture(query = { _, after -> incoming?.takeIf { it.createdHostSeq > after } }) { editor, core, _ ->
                val oldDoc = core.current.documentId
                withContext(Dispatchers.Main) {
                    val info = core.current.info()
                    incoming = ReceivedCapture(WorkflowBinding(info.projectId, "received-capture", 1uL, "state-1"), 1uL,
                        "a".repeat(64), "synthetic-session", 3uL, 7u, true)
                    core.current.change(1uL, 0xff00ffffu)
                }
                waitUntil { editor.captureArrivals.state.value != null && editor.canOpenReceivedCapture() }
                withContext(Dispatchers.Main) {
                    core.current.nextBackground = { entered.complete(Unit); release.await() }
                    editor.openReceivedCapture(checkNotNull(incoming))
                }
                withTimeout(5000) { entered.await() }
                withContext(Dispatchers.Main) { editor.brushDialog(true) }
                release.complete(Unit); waitUntil { !editor.busy }
                withContext(Dispatchers.Main) {
                    assertEquals(oldDoc, editor.document?.documentId); assertNotNull(editor.captureArrivals.state.value)
                    assertTrue(editor.showBrush); assertFalse(core.current.closed)
                }
            }
        } finally { release.complete(Unit) }
    }
    @Test fun navigationDuringHeldDisplayKeepsChosenScreenAndPriorDocument(): Unit = runBlocking {
        var incoming: ReceivedCapture? = null
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            fixture(query = { _, after -> incoming?.takeIf { it.createdHostSeq > after } }) { editor, core, _ ->
                val oldDoc = core.current.documentId
                withContext(Dispatchers.Main) { incoming = announce(core); core.current.change(1uL, 0xff00ffffu) }
                waitUntil { editor.captureArrivals.state.value != null && editor.canOpenReceivedCapture() }
                withContext(Dispatchers.Main) {
                    core.current.nextBackground = { entered.complete(Unit); release.await() }
                    editor.openReceivedCapture(checkNotNull(incoming))
                }
                withTimeout(5000) { entered.await() }
                withContext(Dispatchers.Main) { editor.navigate(WorkbenchScreen.Settings) }
                release.complete(Unit); waitUntil { !editor.busy }
                withContext(Dispatchers.Main) {
                    assertEquals(WorkbenchScreen.Settings, editor.screen)
                    assertEquals(oldDoc, editor.document?.documentId)
                    assertNotNull(editor.captureArrivals.state.value); assertFalse(core.current.closed)
                }
            }
        } finally { release.complete(Unit) }
    }

    @Test fun actualPinchContactDefersAdoptionUntilReleaseAndOldMovesCannotMoveNewDocument(): Unit = runBlocking {
        var incoming: ReceivedCapture? = null
        fixture(query = { _, after -> incoming?.takeIf { it.createdHostSeq > after } }) { editor, core, _ ->
            val router = CanvasGestureRouter(editor, 4.0)
            val oldDoc = core.current.documentId
            try {
                withContext(Dispatchers.Main) {
                    router.accept(touch(MotionEvent.ACTION_DOWN, 1, finger(1, 20.0, 100)))
                    router.accept(touch(MotionEvent.ACTION_POINTER_DOWN, 2, finger(1, 20.0, 110), finger(2, 80.0, 110)))
                    incoming = announce(core); core.current.change(1uL, 0xff00ffffu)
                }
                waitUntil { editor.captureArrivals.state.value != null }
                withContext(Dispatchers.Main) {
                    assertFalse(editor.canOpenReceivedCapture())
                    editor.openReceivedCapture(checkNotNull(incoming))
                    router.accept(touch(MotionEvent.ACTION_MOVE, 2, finger(1, 10.0, 120), finger(2, 100.0, 120)))
                    assertEquals(oldDoc, editor.document?.documentId)
                    router.accept(touch(MotionEvent.ACTION_POINTER_UP, 2, finger(1, 10.0, 130), finger(2, 100.0, 130)))
                    assertFalse(editor.canOpenReceivedCapture()) // The first finger is still down.
                    router.accept(touch(MotionEvent.ACTION_UP, 1, finger(1, 10.0, 140)))
                    assertTrue(editor.canOpenReceivedCapture())
                    editor.openReceivedCapture(checkNotNull(incoming))
                }
                waitUntil { !editor.busy && editor.document?.documentId == "received-capture" }
                withContext(Dispatchers.Main) {
                    val published = checkNotNull(editor.scene).camera
                    router.accept(touch(MotionEvent.ACTION_MOVE, 2, finger(1, 0.0, 150), finger(2, 190.0, 150)))
                    assertEquals(published, checkNotNull(editor.scene).camera)
                    assertNull(editor.captureArrivals.state.value)
                }
            } finally { withContext(NonCancellable + Dispatchers.Main) { router.cancel() } }
        }
    }

    @Test fun contactStartedAndCancelledDuringStagingStillInvalidatesLateDisplay(): Unit = runBlocking {
        var incoming: ReceivedCapture? = null
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            fixture(query = { _, after -> incoming?.takeIf { it.createdHostSeq > after } }) { editor, core, _ ->
                val router = CanvasGestureRouter(editor, 4.0)
                val oldDoc = core.current.documentId
                try {
                    withContext(Dispatchers.Main) { incoming = announce(core); core.current.change(1uL, 0xff00ffffu) }
                    waitUntil { editor.captureArrivals.state.value != null && editor.canOpenReceivedCapture() }
                    withContext(Dispatchers.Main) {
                        core.current.nextBackground = { entered.complete(Unit); release.await() }
                        editor.openReceivedCapture(checkNotNull(incoming))
                    }
                    withTimeout(5000) { entered.await() }
                    withContext(Dispatchers.Main) {
                        router.accept(touch(MotionEvent.ACTION_DOWN, 1, finger(1, 20.0, 100)))
                        router.accept(touch(MotionEvent.ACTION_MOVE, 1, finger(1, 50.0, 110)))
                        router.cancel()
                    }
                    release.complete(Unit); waitUntil { !editor.busy }
                    withContext(Dispatchers.Main) {
                        assertEquals(oldDoc, editor.document?.documentId)
                        assertNotNull(editor.captureArrivals.state.value)
                        assertTrue(editor.canOpenReceivedCapture()) // Admission is restored, but the old ticket stayed stale.
                    }
                } finally { withContext(NonCancellable + Dispatchers.Main) { router.cancel() } }
            }
        } finally { release.complete(Unit) }
    }

    @Test fun syntheticEditorIdentityStillCreatesAndReopensARealNativeProject() = runBlocking<Unit> {
        fixture(query = { _, _ -> null }, afterEditorClosed = { app, core ->
            val device = core.current.info().deviceId
            assertTrue("Fixture identity is not canonical", device.matches(Regex("[0-7][0123456789ABCDEFGHJKMNPQRSTVWXYZ]{25}")))
            val actual = workbenchCore()
            val repository = com.visualworkbench.android.editor.ProjectRepository(app, actual)
            repository.initialize()
            assertEquals(device, repository.deviceId)
            val project = repository.createBlank()
            val info = try { project.info() } finally { project.close() }
            val reopened = repository.open(info.projectId)
            try { assertEquals(info.stateHash, reopened.info().stateHash) }
            finally { reopened.close() }
        }) { _, _, _ -> }
    }

    private suspend fun announce(core: FakeCore): ReceivedCapture {
        val info = core.current.info()
        return ReceivedCapture(WorkflowBinding(info.projectId, "received-capture", 1uL, "state-1"), 1uL,
            "a".repeat(64), "synthetic-session", 3uL, 7u, true)
    }
    private fun finger(id: Int, x: Double, time: Long): PenSample =
        PenSample(id, PointerTool.Finger, x, 50.0, time, 1f, 1f, null, null, 0)
    private fun touch(action: Int, pointer: Int, vararg samples: PenSample): PointerPacket =
        PointerPacket(action, pointer, false, samples.maxOf { it.timeMs },
            listOf(PointerFrame(samples.maxOf { it.timeMs }, samples.toList())),
            StylusCapabilities(true, false, false, false, false, false))

    private suspend fun connect(editor: EditorController) {
        withContext(Dispatchers.Main) { editor.pairing.enter() }
        waitUntil { !editor.pairing.working }
        withContext(Dispatchers.Main) { editor.pairing.choosePeer("synthetic-peer"); editor.pairing.sessionAddress = "192.0.2.1:443"; editor.connectCurrent() }
        waitUntil { editor.connection != null && !editor.busy }
    }
    private suspend fun fixture(query: suspend (WorkbenchProject, ULong) -> ReceivedCapture?,
        afterEditorClosed: suspend (IsolatedEditorApplication, FakeCore) -> Unit = { _, _ -> },
        action: suspend (EditorController, FakeCore, FakeLink) -> Unit) {
        val base = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val app = IsolatedEditorApplication(base)
        val events = mutableListOf<String>(); val link = FakeLink(events); val core = FakeCore(events)
        val sessions = object : SessionLifecycleInstrumentedTest.FakeSessions() {
            override suspend fun pairedDevices() = listOf(PairedDevice("synthetic-peer", "a".repeat(64), 1uL, null))
            override suspend fun projectRole(project: WorkbenchProject) = ProjectSessionRole("synthetic-project", "synthetic-device", "synthetic-device", true, 0u, 0u)
            override suspend fun host(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>) = link
        }
        val store = ViewModelStore()
        var editor: EditorController? = null
        try {
            editor = withContext(Dispatchers.Main) { EditorController(app, core, captureArrivalQuery = query) { _, _ -> sessions }.also { store.put("sync", it) } }
            val current = checkNotNull(editor)
            try {
                waitUntil { !current.busy }
                withContext(Dispatchers.Main) { current.viewport(200, 200, 1f); current.newCanvas() }
                waitUntil { !current.busy && current.scene?.background != null }
                action(current, core, link)
            } finally {
                withContext(NonCancellable + Dispatchers.Main) { store.clear() }
                withContext(NonCancellable) { withTimeout(10_000) { checkNotNull(current.scope.coroutineContext[Job]).join() } }
            }
            afterEditorClosed(app, core)
        } finally {
            withContext(NonCancellable + Dispatchers.Main) { store.clear() }
            withContext(NonCancellable) {
                // No files/preferences retire unless every entered editor owner
                // has actually settled. A timeout intentionally retains them.
                editor?.let { withTimeout(10_000) { checkNotNull(it.scope.coroutineContext[Job]).join() } }
                withContext(Dispatchers.IO) { app.close() }
            }
        }
    }
    private suspend fun waitUntil(check: () -> Boolean) = withTimeout(10_000) { while (!withContext(Dispatchers.Main) { check() }) delay(10) }
    private fun sample(x: Double, y: Double, time: Long) = PenSample(1, PointerTool.Pen, x, y, time, .5f, .5f, null, null, 0)

    private class FakeCore(private val events: MutableList<String>) : WorkbenchCore {
        lateinit var current: FakeProject
        private val identities = workbenchCore()
        override fun newId(unixMs: ULong): String = identities.newId(unixMs)
        override fun newDeviceId(): String = identities.newDeviceId()
        override suspend fun create(options: CreateProject): WorkbenchProject = FakeProject(options, events).also { current = it }
        override suspend fun open(path: String): WorkbenchProject = throw CoreFailure(CoreFailureKind.Unsupported)
        override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("unused")
        override fun cameraMatrix(camera: Camera, inverse: Boolean) = if (inverse) Transform(1 / camera.scale, 0.0, 0.0, 1 / camera.scale,
            camera.center.x - camera.viewportWidth / camera.scale / 2, camera.center.y - camera.viewportHeight / camera.scale / 2)
        else Transform(camera.scale, 0.0, 0.0, camera.scale, camera.viewportWidth / 2 - camera.center.x * camera.scale, camera.viewportHeight / 2 - camera.center.y * camera.scale)
        override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>): List<Point> = cameraMatrix(camera, inverse).let { t -> points.map { Point(it.x * t.a + t.e, it.y * t.d + t.f) } }
    }
    private class FakeProject(options: CreateProject, private val events: MutableList<String>) : WorkbenchProject {
        val documentId = options.documentId
        private val layer = options.layerId
        private var revision = ProjectInfo(options.projectId, "Synthetic", options.deviceId, 1uL, false, false, 0uL, "initial", listOf(documentId))
        private var items = emptyList<RenderItem>()
        override val changes = MutableSharedFlow<ProjectChange>(replay = 1)
        var nextRead: (suspend (DocumentSnapshot) -> DocumentSnapshot)? = null
        var stroke: FakeStroke? = null
        var lastEdit: Pair<EditOptions, List<EditCommand>>? = null
        var nextBackground: (suspend () -> Unit)? = null
        var closed = false
        override suspend fun info() = revision
        suspend fun change(sequence: ULong, color: UInt, overlapping: Boolean = false) {
            revision = revision.copy(hostSeq = sequence, stateHash = "state-$sequence")
            items = listOf(RenderItem("synthetic-object", layer, 1.0, "normal", Rect(0.0, 0.0, 10.0, 10.0), Contours(longArrayOf(), longArrayOf(), uintArrayOf()),
                Transform(), ObjectStyle(color, 1.0), Shape.Line(listOf(Point(0.0, 0.0), Point(10.0, 10.0))), false))
            if (overlapping) {
                val bottom = items.single().copy(bounds = Rect(0.0, 0.0, 20.0, 20.0), shape = Shape.Rectangle(Rect(0.0, 0.0, 20.0, 20.0)))
                items = listOf(bottom, bottom.copy(objectId = "overlapping-front", bounds = Rect(5.0, 5.0, 20.0, 20.0),
                    shape = Shape.Rectangle(Rect(5.0, 5.0, 20.0, 20.0)), style = ObjectStyle(0x0000ffffu, 1.0)))
            }
            changes.emit(ProjectChange(sequence, ChangeKind.Committed, revision))
        }
        override suspend fun document(documentId: String): DocumentSnapshot {
            val value = DocumentSnapshot(documentId, "Synthetic", 100u, 100u, 8u, listOf(LayerInfo(layer, "Marks", true, false, 1.0, "normal")), RenderList(revision, items))
            val hook = nextRead; nextRead = null
            return hook?.invoke(value) ?: value
        }
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage { val hook = nextBackground; nextBackground = null; hook?.invoke(); return BackgroundImage(100u, 100u, ByteArray(40_000) { -1 }, "synthetic-asset", 8u, byteArrayOf()) }
        override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = FakeStroke(revision, events).also { stroke = it }
        override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo { lastEdit = options to commands; return revision }
        override suspend fun undoRedo(options: EditOptions, redo: Boolean) = revision
        override suspend fun render(documentId: String, rectangle: Rect?) = RenderList(revision, items)
        override suspend fun export(options: ExportOptions): ExportResult = error("unused")
        override suspend fun close() { closed = true; events += "project-close" }
    }
    private class FakeStroke(private val revision: ProjectInfo, private val events: MutableList<String>) : WorkbenchStroke {
        var committed = false
        private var count = 0uL
        override suspend fun append(batch: SampleBatch): InkUpdate { count += batch.x.size.toULong(); return InkUpdate(batch.sequence, 0uL, count, Contours(longArrayOf(), longArrayOf(), uintArrayOf())) }
        override suspend fun predict(batch: SampleBatch) = InkUpdate(batch.sequence, 0uL, count, Contours(longArrayOf(), longArrayOf(), uintArrayOf()))
        override suspend fun commit(): ProjectInfo { committed = true; events += "commit"; return revision }
        override fun cancel() = Unit
        override suspend fun dispose() = Unit
    }
    private class FakeLink(val events: MutableList<String>) : ProjectLink {
        override val endpoints = listOf(SessionEndpoint(SessionCarrier.QuicWifi, "192.0.2.1:443"))
        val updates = MutableStateFlow(SessionStatus(1uL, SyncStatus.Synced, SessionCarrier.QuicWifi, 0u, 0u, 0uL, "initial", "synthetic-peer", null, null, 0u, null, null, null))
        override val changes = updates
        override fun status() = updates.value
        val batches = mutableListOf<Triple<WorkbenchStroke, SampleBatch, UInt>>()
        val finishes = mutableListOf<Pair<String, Boolean>>()
        val newObjects = mutableListOf<NewObjectPreview>()
        var preview = PeerPreviews(0uL, emptyList(), emptyList())
        var nextPreview: (suspend () -> PeerPreviews)? = null
        var completedQueries = 0
        override fun sendViewport(documentId: String, corners: List<Point>) = Unit
        override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) { batches += Triple(stroke, batch, firstSample) }
        override fun streamObject(preview: ObjectPreview) = Unit
        override fun streamNewObject(preview: NewObjectPreview) { newObjects += preview }
        override suspend fun finishPreview(gestureId: String, cancel: Boolean) { finishes += gestureId to cancel; events += "finish:$cancel" }
        override suspend fun peerPreviews(documentId: String): PeerPreviews {
            val hook = nextPreview; nextPreview = null
            return try { hook?.invoke() ?: preview } finally { completedQueries++ }
        }
        override suspend fun close() { events += "link-close" }
    }
}
