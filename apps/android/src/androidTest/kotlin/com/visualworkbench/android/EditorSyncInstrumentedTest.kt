@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import android.app.Application
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

/** Real controller/scene lifecycle with synthetic native contracts. These tests
 * are not network, renderer parity, latency or physical-pen acceptance. */
@RunWith(AndroidJUnit4::class)
class EditorSyncInstrumentedTest {
    @Test fun remoteChangesRefreshWithoutMovingCameraAndLateSnapshotCannotOverwrite() = runBlocking {
        fixture { editor, core, _ ->
            val project = core.current
            val camera = withContext(Dispatchers.Main) { checkNotNull(editor.scene).camera }
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            withContext(Dispatchers.Main) {
                project.nextRead = { captured -> entered.complete(Unit); release.await(); captured }
                project.change(1uL, 0xff0000ffu)
            }
            withTimeout(5000) { entered.await() }
            withContext(Dispatchers.Main) { project.change(2uL, 0x00ff00ffu) }
            release.complete(Unit)
            waitUntil { editor.info?.stateHash == "state-2" }
            withContext(Dispatchers.Main) {
                assertEquals(0x00ff00ffu, editor.document?.render?.items?.single()?.style?.rgba)
                assertEquals(camera, editor.scene?.camera)
            }
        }
    }

    @Test fun nativeStrokeHandleStreamsRealBatchesOnlyAndFinishesAfterDurableCommit() = runBlocking {
        fixture { editor, core, link ->
            connect(editor)
            val first = sample(10.0, 10.0, 1000)
            withContext(Dispatchers.Main) { editor.beginTool(first, StylusCapabilities(true, false, false, false, false, false)) }
            waitUntil { core.current.stroke != null }
            withContext(Dispatchers.Main) {
                editor.predict(sample(99.0, 99.0, 1040))
                editor.extendTool(listOf(sample(20.0, 20.0, 1010)))
            }
            waitUntil { link.batches.size >= 2 }
            withContext(Dispatchers.Main) { editor.finishTool(listOf(sample(30.0, 30.0, 1020))) }
            waitUntil { editor.pending == 0 && link.finishes.any { !it.second } }
            withContext(Dispatchers.Main) {
                val stroke = checkNotNull(core.current.stroke)
                assertTrue(link.batches.all { it.first === stroke })
                assertFalse(link.batches.any { (_, batch, _) -> batch.timeMs.contains(40u) })
                assertEquals(listOf(0u, 1u, 2u), link.batches.map { it.third })
                assertTrue(link.events.indexOf("commit") < link.events.indexOf("finish:false"))
            }
        }
    }

    @Test fun backgroundCancelsPreviewAndProjectSwapClosesLinkBeforeProject() = runBlocking {
        fixture { editor, core, link ->
            connect(editor)
            withContext(Dispatchers.Main) { editor.beginTool(sample(10.0, 10.0, 2000), StylusCapabilities(true, false, false, false, false, false)) }
            waitUntil { link.batches.isNotEmpty() }
            withContext(Dispatchers.Main) { editor.onBackground() }
            waitUntil { editor.pending == 0 && link.finishes.any { it.second } }
            assertFalse(core.current.stroke?.committed ?: true)
            val old = core.current
            withContext(Dispatchers.Main) { editor.navigate(WorkbenchScreen.Projects) }
            waitUntil { old.closed }
            assertTrue(link.events.indexOf("link-close") >= 0)
            assertTrue(link.events.indexOf("link-close") < link.events.indexOf("project-close"))
        }
    }

    @Test fun peerViewportRemainsIndependentUntilExplicitMatchOrFollow() = runBlocking {
        fixture { editor, core, link ->
            connect(editor)
            val before = withContext(Dispatchers.Main) { checkNotNull(editor.scene).camera }
            withContext(Dispatchers.Main) {
                link.updates.value = link.status().copy(sequence = 2uL, peerViewport = PeerViewport(core.current.documentId,
                    listOf(Point(20.0, 30.0), Point(60.0, 30.0), Point(60.0, 70.0), Point(20.0, 70.0))))
            }
            waitUntil { editor.connection?.sequence == 2uL }
            withContext(Dispatchers.Main) {
                assertEquals(before, editor.scene?.camera)
                editor.matchPeer()
                assertEquals(Point(40.0, 50.0), editor.scene?.camera?.center)
                assertFalse(editor.followPeer)
                editor.followPeer(true); editor.pan(1.0, 2.0); assertFalse(editor.followPeer)
            }
        }
    }

    @Test fun newShapePreviewKeepsIdentityAndWidthAndCommitsItsFinalGeometry() = runBlocking {
        fixture { editor, core, link ->
            connect(editor)
            withContext(Dispatchers.Main) {
                editor.chooseTool(EditorTool.Rectangle)
                editor.beginTool(sample(10.0, 10.0, 3000), StylusCapabilities(true, false, false, false, false, false))
                editor.extendTool(listOf(sample(30.0, 25.0, 3010)))
                editor.extendTool(listOf(sample(80.0, 50.0, 3020)))
                editor.finishTool(listOf(sample(90.0, 60.0, 3030)))
            }
            waitUntil { editor.pending == 0 && core.current.lastEdit != null }
            withContext(Dispatchers.Main) {
                assertTrue(link.newObjects.size >= 3)
                val final = link.newObjects.last()
                val edit = checkNotNull(core.current.lastEdit)
                val created = edit.second.single() as EditCommand.Create
                assertEquals(1, link.newObjects.map { it.objectId }.distinct().size)
                assertEquals(1, link.newObjects.map { it.gestureId }.distinct().size)
                assertEquals(1, link.newObjects.map { it.style.width }.distinct().size)
                assertTrue(link.newObjects.all { it.transform == Transform() })
                assertEquals(final.shape, created.shape); assertEquals(final.style, created.style)
                assertEquals(final.objectId, created.objectId); assertEquals(final.gestureId, edit.first.gestureId)
                assertEquals(final.createdAtMs, edit.first.createdAtMs)
            }
        }
    }

    @Test fun delayedPeerQueryCannotRestoreAnOverlayAfterAnAuthoritativeRefresh() = runBlocking {
        fixture { editor, core, link ->
            withContext(Dispatchers.Main) { core.current.change(1uL, 0xff0000ffu) }
            waitUntil { editor.info?.stateHash == "state-1" }
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            val observed = mutableListOf<UInt>()
            withContext(Dispatchers.Main) {
                val item = checkNotNull(editor.document).render.items.single().copy(style = ObjectStyle(0xffff00ffu, 1.0))
                link.nextPreview = { entered.complete(Unit); release.await(); PeerPreviews(9uL, listOf("stale"), listOf(item)) }
            }
            connect(editor)
            withTimeout(5000) { entered.await() }
            withContext(Dispatchers.Main) { core.current.change(2uL, 0x00ff00ffu) }
            waitUntil { editor.info?.stateHash == "state-2" }
            withContext(Dispatchers.Main) {
                editor.attachCanvas { scene -> observed += (scene.objects + scene.provisional).map { it.item.style.rgba } }
            }
            release.complete(Unit)
            waitUntil { link.completedQueries >= 2 }
            withContext(Dispatchers.Main) {
                assertFalse("A late peer query hid the newer canonical object", observed.contains(0xffff00ffu))
                assertEquals(0x00ff00ffu, editor.scene?.objects?.single()?.item?.style?.rgba)
                editor.detachCanvas()
            }
        }
    }

    @Test fun delayedPeerQueryCannotPublishAcrossOfflineAndReconnectStatusGenerations() = runBlocking {
        for (status in listOf(SyncStatus.Offline, SyncStatus.Reconnecting)) fixture { editor, core, link ->
            withContext(Dispatchers.Main) { core.current.change(1uL, 0xff0000ffu) }
            waitUntil { editor.info?.stateHash == "state-1" }
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            val observed = mutableListOf<UInt>()
            withContext(Dispatchers.Main) {
                val item = checkNotNull(editor.document).render.items.single().copy(style = ObjectStyle(0xffff00ffu, 1.0))
                link.nextPreview = { entered.complete(Unit); release.await(); PeerPreviews(9uL, listOf("stale"), listOf(item)) }
            }
            connect(editor)
            withTimeout(5000) { entered.await() }
            withContext(Dispatchers.Main) { link.updates.value = link.status().copy(sequence = 2uL, status = status) }
            waitUntil { editor.connection?.sequence == 2uL }
            // Even a complete reconnect before the old query settles must not
            // allow that old result into the new connected generation.
            withContext(Dispatchers.Main) {
                link.updates.value = link.status().copy(sequence = 3uL, status = SyncStatus.Synced)
                editor.attachCanvas { scene -> observed += (scene.objects + scene.provisional).map { it.item.style.rgba } }
            }
            waitUntil { editor.connection?.sequence == 3uL }
            release.complete(Unit)
            waitUntil { link.completedQueries >= 2 }
            withContext(Dispatchers.Main) {
                assertFalse("A pre-$status query reappeared after reconnect", observed.contains(0xffff00ffu))
                editor.detachCanvas()
            }
        }
    }

    @Test fun observedCommitClearsThePreviousOverlayBeforeSnapshotPreparationCompletes() = runBlocking {
        fixture { editor, core, link ->
            withContext(Dispatchers.Main) { core.current.change(1uL, 0xff0000ffu) }
            waitUntil { editor.info?.stateHash == "state-1" }
            withContext(Dispatchers.Main) {
                val item = checkNotNull(editor.document).render.items.single()
                link.preview = PeerPreviews(1uL, listOf("preview"), listOf(item.copy(style = ObjectStyle(0xffff00ffu, 1.0))))
            }
            connect(editor)
            waitUntil { editor.scene?.objects?.singleOrNull()?.item?.style?.rgba == 0xffff00ffu }
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            try {
                withContext(Dispatchers.Main) {
                    link.preview = PeerPreviews(2uL, emptyList(), emptyList())
                    core.current.nextRead = { captured -> entered.complete(Unit); release.await(); captured }
                    core.current.change(2uL, 0x00ff00ffu)
                }
                withTimeout(5000) { entered.await() }
                withContext(Dispatchers.Main) {
                    assertEquals("state-1", editor.info?.stateHash)
                    assertEquals(0xff0000ffu, editor.scene?.objects?.single()?.item?.style?.rgba)
                }
            } finally { release.complete(Unit) }
            waitUntil { editor.info?.stateHash == "state-2" }
        }
    }

    @Test fun existingPeerPreviewKeepsCanonicalStackingOrderAndOnlyNewIdsAppend() = runBlocking {
        fixture { editor, core, link ->
            withContext(Dispatchers.Main) { core.current.change(1uL, 0xff0000ffu, overlapping = true) }
            waitUntil { editor.info?.stateHash == "state-1" }
            withContext(Dispatchers.Main) {
                val bottom = checkNotNull(editor.document).render.items.first()
                link.preview = PeerPreviews(1uL, listOf("drag", "new"), listOf(
                    bottom.copy(style = bottom.style.copy(rgba = 0x00ff00ffu)),
                    bottom.copy(objectId = "new-peer-object")))
            }
            connect(editor)
            waitUntil { editor.scene?.provisional?.any { it.item.objectId == "new-peer-object" } == true }
            withContext(Dispatchers.Main) {
                val scene = checkNotNull(editor.scene)
                assertEquals(listOf("synthetic-object", "overlapping-front"), scene.objects.map { it.item.objectId })
                assertEquals(0x00ff00ffu, scene.objects.first().item.style.rgba)
                assertEquals(0x0000ffffu, scene.objects.last().item.style.rgba)
                assertEquals(listOf("new-peer-object"), scene.provisional.map { it.item.objectId })
            }
        }
    }

    @Test fun grantIndicatorUsesActualLinkCollectorAndRetiresBeforeProjectClose() = runBlocking {
        val api = CaptureStatusFake()
        fixture(capture = { api }) { editor, core, link ->
            connect(editor)
            withContext(Dispatchers.Main) { api.emit(AgentCaptureStatus(2uL, 1uL, true, 2u, false, 30_000u)) }
            waitUntil { editor.agentCapture.state.value.activeGrantCount == 2u }
            withContext(Dispatchers.Main) {
                assertTrue(agentCaptureStatusText(editor.agentCapture.state.value).contains("enabled"))
                link.updates.value = link.status().copy(sequence = 2uL, status = SyncStatus.Offline)
            }
            waitUntil { editor.connection?.status == SyncStatus.Offline }
            withContext(Dispatchers.Main) { assertFalse(editor.agentCapture.state.value.known) }
            val prior = core.current
            withContext(Dispatchers.Main) { editor.navigate(WorkbenchScreen.Projects) }
            waitUntil { prior.closed }
            withContext(Dispatchers.Main) {
                assertFalse(editor.agentCapture.state.value.known)
                assertEquals(0, api.activeWaits)
                assertTrue(link.events.indexOf("link-close") < link.events.indexOf("project-close"))
            }
        }
    }
    @Test fun grantIndicatorReplacesSnapshotsAndKeepsRevokedCaptureVisibleWhileItSettles() = runBlocking {
        val api = CaptureStatusFake()
        fixture(capture = { api }) { editor, _, _ ->
            connect(editor)
            for (value in listOf(AgentCaptureStatus(2uL, 1uL, true, 3u, true, 10_000u),
                AgentCaptureStatus(3uL, 1uL, true, 0u, true, 0u), AgentCaptureStatus(4uL, 1uL, true))) {
                withContext(Dispatchers.Main) { api.emit(value) }
                waitUntil { editor.agentCapture.state.value == value }
            }
            withContext(Dispatchers.Main) {
                assertEquals("PC agent capture: no active grants", agentCaptureStatusText(editor.agentCapture.state.value))
                assertEquals(0, api.publications)
            }
        }
    }
    private class CaptureStatusFake : WorkbenchAgentCaptureStatus {
        private val updates = kotlinx.coroutines.channels.Channel<AgentCaptureStatus>(kotlinx.coroutines.channels.Channel.CONFLATED)
        private var current = AgentCaptureStatus(1uL, 1uL)
        var activeWaits = 0; var publications = 0
        fun emit(value: AgentCaptureStatus) { current = value; updates.trySend(value) }
        override fun status() = current
        override suspend fun waitStatus(afterSequence: ULong): AgentCaptureStatus {
            activeWaits++
            try { while (true) { val value = updates.receive(); if (value.sequence > afterSequence) return value } }
            finally { activeWaits-- }
        }
        override fun publish(value: LocalAgentCaptureSummary) { publications++; error("Phone indicator cannot publish grants") }
    }

    private suspend fun connect(editor: EditorController) {
        withContext(Dispatchers.Main) { editor.pairing.enter() }
        waitUntil { !editor.pairing.working }
        withContext(Dispatchers.Main) { editor.pairing.choosePeer("synthetic-peer"); editor.pairing.sessionAddress = "192.0.2.1:443"; editor.connectCurrent() }
        waitUntil { editor.connection != null && !editor.busy }
    }
    private suspend fun fixture(capture: (ProjectLink) -> WorkbenchAgentCaptureStatus = ::agentCaptureStatus, action: suspend (EditorController, FakeCore, FakeLink) -> Unit) {
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
            editor = withContext(Dispatchers.Main) { EditorController(app, core, capture) { _, _ -> sessions }.also { store.put("sync", it) } }
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
        var closed = false
        override suspend fun info() = revision
        suspend fun change(sequence: ULong, color: UInt, overlapping: Boolean = false) {
            revision = revision.copy(stateHash = "state-$sequence")
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
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean) = BackgroundImage(100u, 100u, ByteArray(40_000) { -1 }, "synthetic-asset", 8u, byteArrayOf())
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
