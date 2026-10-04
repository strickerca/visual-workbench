package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path

class SelectionControllerTest {
    @Test fun pickerKeepsExactMaskAndPublishesOnlyCompletedBytes() = runBlocking {
        fixture { controller, api, root ->
            val request = checkNotNull(controller.ticket(SelectionExportKind.Cutout, true))
            val target = root.resolve("cutout.png")
            assertTrue(controller.save(request, target))
            withTimeout(5000) { controller.saved.first { !it.saving && it.completed != null } }
            assertArrayEquals(byteArrayOf(1, 2, 3, 4), Files.readAllBytes(target))
            assertEquals(request.binding, api.exported!!.binding)
            assertEquals(request.selection, api.exported!!.selection)
            assertEquals(request.region, api.exported!!.region)
            Files.delete(target)
            assertEquals(0L, Files.list(root.resolve("private")).use { it.count() })
        }
    }
    @Test fun changedRevisionDuringExportNeverPublishesDestination() = runBlocking {
        fixture { controller, api, root ->
            api.pause = CompletableDeferred()
            val request = checkNotNull(controller.ticket(SelectionExportKind.Mask, false))
            val target = root.resolve("mask.png"); controller.save(request, target)
            withTimeout(5000) { api.entered.await() }
            controller.observe(ProjectChange(2u, ChangeKind.Committed, selectionTestInfo.copy(stateHash = "pending-new")))
            api.pause!!.complete(Unit)
            withTimeout(5000) { controller.saved.first { !it.saving } }
            assertFalse(Files.exists(target)); assertNull(controller.saved.value.completed)
            assertEquals(0L, Files.list(root.resolve("private")).use { it.count() })
        }
    }
    @Test fun cancellationSettlesNativeProducerBeforePrivateCleanup() = runBlocking {
        fixture { controller, api, root ->
            api.pause = CompletableDeferred(); api.settlement = CompletableDeferred()
            controller.save(checkNotNull(controller.ticket(SelectionExportKind.Mask, false)), root.resolve("mask.png"))
            withTimeout(5000) { api.entered.await() }; controller.cancel()
            withTimeout(5000) { api.cancelled.await() }
            assertTrue(Files.exists(Path.of(api.exported!!.outputPath)))
            assertTrue(controller.saved.value.saving)
            api.settlement!!.complete(Unit)
            withTimeout(5000) { controller.saved.first { !it.saving } }
            assertEquals(0L, Files.list(root.resolve("private")).use { it.count() })
            assertFalse(Files.exists(root.resolve("mask.png")))
        }
    }
    @Test fun displayTintHasExactIndependentCoverageEndpoints() {
        val values = selectionArgb(byteArrayOf(0, 128.toByte(), 255.toByte()))
        assertEquals(listOf(0, 55, 110), values.map { it ushr 24 })
        assertTrue(values.all { it and 0x00ffffff == 0x00d15bea })
    }
    private suspend fun fixture(block: suspend (SelectionController, SelectionExportFixture, Path) -> Unit) = coroutineScope {
        val root = Files.createTempDirectory("vw-selection-controller-")
        val owner = CoroutineScope(coroutineContext + SupervisorJob(coroutineContext[Job]))
        val api = SelectionExportFixture()
        val controller = SelectionController(owner, SelectionTestCore(), root.resolve("private"), {}, { api })
        try {
            controller.bind(SelectionTestProject(), selectionTestDocument(), 1L, 1u, Camera(Point(32.0, 32.0), 1.0, 0.0, 64.0, 64.0))
            assertNull(controller.state.value.document) // Capability stays lazy until tool activation.
            controller.interaction.useObject("mask-object")
            withTimeout(5000) { controller.state.first { it.document != null && it.selection != null && !it.loading } }
            block(controller, api, root)
        } finally {
            withContext(NonCancellable) { controller.detach() }; owner.cancel()
            if (Files.exists(root.resolve("private"))) Files.delete(root.resolve("private"))
            Files.delete(root)
        }
    }
}
private val selectionTestInfo = ProjectInfo("project", "Selection fixture", "device", 1u, false, false, 0u, "hash", listOf("doc"))
private fun selectionTestDocument() = DocumentSnapshot("doc", "Fixture", 64u, 64u, 8u, emptyList(), RenderList(selectionTestInfo, emptyList()))
private class SelectionExportFixture : WorkbenchSelections {
    var pause: CompletableDeferred<Unit>? = null
    var settlement: CompletableDeferred<Unit>? = null
    val entered = CompletableDeferred<Unit>(); val cancelled = CompletableDeferred<Unit>()
    var exported: SelectionExportOptions? = null
    private val binding = SelectionBinding("project", "doc", "source", 0u, "hash")
    override suspend fun document(documentId: String) = SelectionDocument(binding, 64u, 64u, 8u, selectionTestInfo)
    override suspend fun snapshot(binding: SelectionBinding, objectId: String, memoryBudgetBytes: ULong) = SelectionSnapshot(binding,
        SelectionVersion(objectId, "coverage", 1u), "layer", 64u, 64u, SelectionRegion(2u, 3u, 8u, 9u), true, true)
    override suspend fun tiles(binding: SelectionBinding, selection: SelectionVersion, regions: List<SelectionRegion>, memoryBudgetBytes: ULong) =
        SelectionTiles(binding, selection, regions.map { SelectionTile(it, ByteArray((it.width * it.height).toInt())) })
    override suspend fun edit(options: SelectionEdit): SelectionReceipt = error("Unused")
    override suspend fun exportFile(options: SelectionExportOptions): SelectionExportReceipt {
        exported = options; Files.write(Path.of(options.outputPath), byteArrayOf(1, 2, 3, 4)); entered.complete(Unit)
        try { pause?.await() }
        catch (error: CancellationException) { cancelled.complete(Unit); withContext(NonCancellable) { settlement?.await() }; throw error }
        return SelectionExportReceipt(options.binding, options.selection, options.kind,
            options.region ?: SelectionRegion(0u, 0u, 64u, 64u), 8u, 4u, "0".repeat(64), "{}", selectionTestInfo, 1024u)
    }
}
private class SelectionTestCore : WorkbenchCore {
    override fun newId(unixMs: ULong) = "id-$unixMs"
    override fun newDeviceId() = "device"
    override fun cameraMatrix(camera: Camera, inverse: Boolean) = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>) = points
    override suspend fun create(options: CreateProject): WorkbenchProject = error("Unused")
    override suspend fun open(path: String): WorkbenchProject = error("Unused")
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("Unused")
}
private class SelectionTestProject : WorkbenchProject {
    override val changes = emptyFlow<ProjectChange>()
    override suspend fun info() = selectionTestInfo
    override suspend fun document(documentId: String) = selectionTestDocument()
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("Unused")
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("Unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Unused")
    override suspend fun render(documentId: String, rectangle: Rect?) = selectionTestDocument().render
    override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
    override suspend fun close() = Unit
}
