@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.io.IOException
import java.nio.file.FileVisitResult
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.SimpleFileVisitor
import java.nio.file.attribute.BasicFileAttributes
import java.util.ArrayDeque
import java.util.concurrent.Executors

class DesktopBackgroundDisplayTest {
    @Test fun displayRefusalsKeepTheAdoptedAttachmentAndSpecificFailureKind() = backgroundFixture { f ->
        val issues = mutableSetOf<String>()
        for (kind in CoreFailureKind.entries) {
            val project = BackgroundProject().apply { next { throw CoreFailure(kind, IllegalStateException("private-source-detail")) } }
            val state = f.open(project)
            assertSame(project.document, state.document)
            assertEquals(f.path, state.path)
            assertNull(state.background)
            assertNull(state.renderIssue)
            assertEquals(kind, state.backgroundFailureKind)
            assertEquals(kind == CoreFailureKind.Raster, state.needsColorConsent)
            val issue = checkNotNull(state.backgroundIssue)
            assertTrue(issue.length in 1..256)
            assertFalse(issue.contains("private-source-detail"))
            assertTrue(issues.add(issue))
            assertEquals(listOf(BackgroundRequest("document", DISPLAY_BUDGET, false)), project.requests)
            assertEquals(0, project.closed)
        }
    }

    @Test fun explicitRetryKeepsTheProjectAndFixedBudgetAndClearsTheIssueOnSuccess() = backgroundFixture { f ->
        val gate = BackgroundGate()
        val project = BackgroundProject().apply {
            next { throw CoreFailure(CoreFailureKind.Backpressure) }
            next { gate.waitForRelease(); pixels() }
        }
        val refused = f.open(project)
        val retry = checkNotNull(f.editor.retryBackground())
        gate.entered.await()
        assertTrue(f.editor.state.value.busy)
        assertNull(f.editor.state.value.backgroundIssue)
        assertNull(f.editor.state.value.backgroundFailureKind)
        assertEquals(refused.projectEpoch, f.editor.state.value.projectEpoch)
        gate.release.complete(Unit)
        retry.join()
        val ready = f.idle { it.background != null }
        assertSame(project.document, ready.document)
        assertEquals(refused.projectEpoch, ready.projectEpoch)
        assertEquals(1, f.core.opened.size)
        assertEquals(List(2) { BackgroundRequest("document", DISPLAY_BUDGET, false) }, project.requests)
        val image = checkNotNull(ready.background)
        assertEquals(64, image.width)
        assertEquals(64, image.height)
        assertNull(ready.backgroundIssue)
        assertNull(ready.backgroundFailureKind)
        assertFalse(ready.needsColorConsent)
        assertEquals(0, project.closed)
    }

    @Test fun onlyExplicitSrgbConsentChangesTheAssumptionForTheCurrentAttachment() = backgroundFixture { f ->
        val project = BackgroundProject().apply {
            next { throw CoreFailure(CoreFailureKind.Raster) }
            next { throw CoreFailure(CoreFailureKind.Raster) }
            next { pixels() }
            next { pixels() }
        }
        val refused = f.open(project)
        assertTrue(refused.needsColorConsent)
        checkNotNull(f.editor.retryBackground()).join()
        assertTrue(f.idle().needsColorConsent)
        f.editor.assumeSrgb()
        val ready = f.idle { project.requests.size == 3 && it.background != null }
        assertEquals(refused.projectEpoch, ready.projectEpoch)
        assertSame(project.document, ready.document)
        assertFalse(ready.needsColorConsent)
        assertNull(ready.backgroundIssue)
        assertNull(ready.backgroundFailureKind)
        checkNotNull(f.editor.retryBackground()).join()
        f.idle()
        assertEquals(listOf(false, false, true, true), project.requests.map { it.assumedSrgb })
        assertTrue(project.requests.all { it.budget == DISPLAY_BUDGET && it.documentId == "document" })
        assertEquals(listOf(project), f.core.opened)
    }

    @Test fun reopeningTheSameProjectResetsTheIssueAndExplicitColorConsent() = backgroundFixture { f ->
        val first = BackgroundProject().apply {
            next { throw CoreFailure(CoreFailureKind.Raster) }
            next { pixels() }
            next { throw CoreFailure(CoreFailureKind.Worker) }
        }
        f.open(first)
        f.editor.assumeSrgb()
        f.idle { first.requests.size == 2 && it.background != null }
        checkNotNull(f.editor.retryBackground()).join()
        val prior = f.idle()
        assertEquals(CoreFailureKind.Worker, prior.backgroundFailureKind)
        val gate = BackgroundGate()
        val reopened = BackgroundProject().apply { next { gate.waitForRelease(); throw CoreFailure(CoreFailureKind.Raster) } }
        f.enqueueOpen(reopened)
        gate.entered.await()
        val loading = f.editor.state.value
        assertTrue(loading.projectEpoch > prior.projectEpoch)
        assertSame(reopened.document, loading.document)
        assertNull(loading.background)
        assertNull(loading.backgroundIssue)
        assertNull(loading.backgroundFailureKind)
        assertFalse(loading.needsColorConsent)
        assertEquals(1, first.closed)
        assertEquals(listOf(BackgroundRequest("document", DISPLAY_BUDGET, false)), reopened.requests)
        gate.release.complete(Unit)
        val refused = f.idle { it.backgroundFailureKind == CoreFailureKind.Raster }
        assertTrue(refused.needsColorConsent)
        assertNotEquals(prior.backgroundIssue, refused.backgroundIssue)
        f.editor.closeProject()
        val closed = f.idle { it.document == null }
        assertNull(closed.backgroundIssue)
        assertNull(closed.backgroundFailureKind)
        assertFalse(closed.needsColorConsent)
        assertEquals(1, reopened.closed)
    }

    @Test fun roleRefreshAndGeneralMessagesCannotEraseTheBackgroundIssue() = backgroundFixture { f ->
        val project = BackgroundProject().apply { next { throw CoreFailure(CoreFailureKind.Backpressure) } }
        val refused = f.open(project)
        val role = ProjectSessionRole("project", "owner", "owner", true, 3u, 1u)
        val service = object : SessionFixture() {
            var roleReads = 0
            override suspend fun projectRole(value: WorkbenchProject): ProjectSessionRole {
                assertSame(project, value)
                roleReads++
                return role
            }
        }.apply { device = "owner" }
        val sessions = DesktopSessionController(f.scope, "owner", { service })
        try {
            sessions.requireService()
            f.editor.report("Pairing status")
            f.editor.inspectSessionRole(sessions)
            val inspected = f.idle { it.sessionRole == role }
            assertEquals(1, service.roleReads)
            assertEquals(refused.backgroundIssue, inspected.backgroundIssue)
            assertEquals(CoreFailureKind.Backpressure, inspected.backgroundFailureKind)
            f.editor.report("Project connection status")
            f.editor.clearMessage()
            assertNull(f.editor.state.value.message)
            assertEquals(refused.backgroundIssue, f.editor.state.value.backgroundIssue)
            assertEquals(CoreFailureKind.Backpressure, f.editor.state.value.backgroundFailureKind)
        } finally { sessions.close() }
        assertEquals(1, service.closed)
    }

    @Test fun ordinaryRequestAndBitmapFailuresRemainNeutralAndBounded() = backgroundFixture { f ->
        val requestFailure = BackgroundProject().apply {
            next { throw IllegalStateException("private-source-detail") }
            next { throw CoreFailure(CoreFailureKind.Raster) }
            next { throw IllegalStateException("private-source-detail") }
        }
        val first = f.open(requestFailure)
        checkNotNull(f.editor.retryBackground()).join()
        assertTrue(f.idle().needsColorConsent)
        checkNotNull(f.editor.retryBackground()).join()
        val retried = f.idle()
        assertTrue(retried.needsColorConsent)
        assertNull(retried.backgroundFailureKind)
        assertEquals(first.backgroundIssue, retried.backgroundIssue)
        val malformedPixels = BackgroundProject().apply { next { BackgroundImage(1u, 1u, byteArrayOf(), "source", 8u, byteArrayOf()) } }
        val second = f.open(malformedPixels)
        for (state in listOf(first, second)) {
            assertNull(state.background)
            assertNull(state.backgroundFailureKind)
            assertNull(state.renderIssue)
            assertFalse(state.needsColorConsent)
            val issue = checkNotNull(state.backgroundIssue)
            assertTrue(issue.length in 1..256)
            assertFalse(issue.contains("private-source-detail"))
        }
        assertEquals(first.backgroundIssue, second.backgroundIssue)
        assertSame(malformedPixels.document, second.document)
        assertEquals(0, malformedPixels.closed)
    }

    @Test fun coroutineCancellationWaitsForItsProducerAndNeverPublishesAnImageOrRefusal() = backgroundFixture { f ->
        val gate = BackgroundGate()
        var producerSettled = false
        val project = BackgroundProject().apply {
            next { throw CoreFailure(CoreFailureKind.Raster) }
            next { throw CancellationException("private-source-detail") }
            next {
                withContext(NonCancellable) { gate.waitForRelease() }
                producerSettled = true
                pixels()
            }
        }
        try {
            val refused = f.open(project)
            val cancelledRequest = checkNotNull(f.editor.retryBackground())
            cancelledRequest.join()
            assertTrue(cancelledRequest.isCancelled)
            assertNull(f.editor.state.value.backgroundIssue)
            assertNull(f.editor.state.value.backgroundFailureKind)
            val retry = checkNotNull(f.editor.retryBackground())
            gate.entered.await()
            project.beforeClose = { assertTrue(producerSettled) }
            retry.cancel()
            yield()
            assertTrue(f.editor.state.value.busy)
            assertFalse(retry.isCompleted)
            assertEquals(0, project.closed)
            gate.release.complete(Unit)
            retry.join()
            val settled = f.idle()
            assertTrue(retry.isCancelled)
            assertTrue(producerSettled)
            assertEquals(refused.projectEpoch, settled.projectEpoch)
            assertSame(project.document, settled.document)
            assertNull(settled.background)
            assertNull(settled.backgroundIssue)
            assertNull(settled.backgroundFailureKind)
            f.editor.closeProject()
            f.idle { it.document == null }
            assertEquals(1, project.closed)
        } finally { gate.release.complete(Unit) }
    }
}

private val DISPLAY_BUDGET = 256uL * 1024uL * 1024uL
private data class BackgroundRequest(val documentId: String, val budget: ULong, val assumedSrgb: Boolean)
private class BackgroundGate {
    val entered = CompletableDeferred<Unit>()
    val release = CompletableDeferred<Unit>()
    suspend fun waitForRelease() { entered.complete(Unit); release.await() }
}
private class BackgroundProject : WorkbenchProject {
    val document = DocumentSnapshot("document", "fixture", 64u, 64u, 8u,
        listOf(LayerInfo("layer", "Annotations", true, false, 1.0, "normal")),
        RenderList(ProjectInfo("project", "fixture", "owner", 8u, true, false, 7u, "hash-7", listOf("document")), emptyList()))
    private val responses = ArrayDeque<suspend () -> BackgroundImage>()
    val requests = mutableListOf<BackgroundRequest>()
    var closed = 0
    var beforeClose: (() -> Unit)? = null
    override val changes = MutableSharedFlow<ProjectChange>()
    fun next(response: suspend () -> BackgroundImage) { responses.addLast(response) }
    override suspend fun info() = document.render.revision
    override suspend fun document(documentId: String) = document
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage {
        check(closed == 0)
        requests += BackgroundRequest(documentId, memoryBudgetBytes, assumeUntaggedSrgb)
        return responses.removeFirst().invoke()
    }
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
    override suspend fun render(documentId: String, rectangle: Rect?) = document.render
    override suspend fun export(options: ExportOptions): ExportResult = error("unused")
    override suspend fun close() { beforeClose?.invoke(); closed++ }
}
private class BackgroundCore : WorkbenchCore {
    val pending = ArrayDeque<BackgroundProject>()
    val opened = mutableListOf<BackgroundProject>()
    private var serial = 0
    override fun newId(unixMs: ULong) = "background-test-${serial++}"
    override fun newDeviceId() = error("fixture must never request an installation identity")
    override suspend fun create(options: CreateProject): WorkbenchProject = error("unused")
    override suspend fun open(path: String): WorkbenchProject = pending.removeFirst().also { opened += it }
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("unused")
    override fun cameraMatrix(camera: Camera, inverse: Boolean) = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>) = points
}
private class BackgroundFixture(root: Path, val scope: CoroutineScope) {
    val path: Path = root.resolve("project.sqlite")
    val core = BackgroundCore()
    val editor = EditorController(scope, core, object : DesktopHandoff {
        override suspend fun paste(assumeSrgb: Boolean): ClipboardImage = error("fixture must never use the real clipboard")
        override suspend fun copy(path: Path, receipt: CompletedExport): Unit = error("fixture must never use the real clipboard")
        override suspend fun stage(path: Path, receipt: CompletedExport): DesktopDragFile = error("fixture must never create a real drag")
    }, LocationPolicy(root.resolve("projects"), false, "isolated test"), "owner", root.resolve("private"))
    fun enqueueOpen(project: BackgroundProject) { core.pending.addLast(project); editor.open(path) }
    suspend fun open(project: BackgroundProject): EditorState {
        val prior = editor.state.value.projectEpoch
        enqueueOpen(project)
        val state = idle { it.projectEpoch > prior && it.document === project.document }
        project.changes.subscriptionCount.first { it > 0 }
        return state
    }
    suspend fun idle(condition: (EditorState) -> Boolean = { true }): EditorState = withTimeout(5000) {
        combine(editor.state, editor.instructionEditor.state, editor.semanticEditor.state) { view, instructions, semantics ->
            view.takeIf { !it.busy && !instructions.busy && !semantics.busy && condition(it) }
        }.first { it != null }!!
    }
}
private fun pixels() = BackgroundImage(64u, 64u, ByteArray(64 * 64 * 4) { if (it % 4 == 3) 255.toByte() else 60 }, "source", 8u, byteArrayOf())
private fun backgroundFixture(block: suspend (BackgroundFixture) -> Unit) {
    val root = Files.createTempDirectory("vw-desktop-background-").toRealPath()
    try {
        Executors.newSingleThreadExecutor { r -> Thread(r, "vw-background-test").apply { isDaemon = true } }.asCoroutineDispatcher().use { dispatcher ->
            runBlocking(dispatcher) {
                val lifetime = SupervisorJob(coroutineContext[Job])
                val fixture = BackgroundFixture(root, CoroutineScope(coroutineContext + lifetime))
                try { withTimeout(15_000) { block(fixture) } }
                finally { withContext(NonCancellable) { lifetime.cancelAndJoin(); fixture.editor.close() } }
            }
        }
    } finally {
        check(root.fileName.toString().startsWith("vw-desktop-background-") && root.toRealPath() == root)
        Files.walkFileTree(root, object : SimpleFileVisitor<Path>() {
            override fun visitFile(file: Path, attributes: BasicFileAttributes): FileVisitResult {
                check(file.startsWith(root)); Files.delete(file); return FileVisitResult.CONTINUE
            }
            override fun postVisitDirectory(dir: Path, error: IOException?): FileVisitResult {
                if (error != null) throw error
                check(dir.startsWith(root)); Files.delete(dir); return FileVisitResult.CONTINUE
            }
        })
    }
}
