package com.visualworkbench.android

import android.content.Context
import android.content.ContextWrapper
import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.SelectionController
import com.visualworkbench.android.editor.SelectionDestination
import com.visualworkbench.android.editor.selectionArgb
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Synthetic provider/native contracts. Never opens another app's provider,
 * clipboard or phone data, and makes no physical-pen/renderer acceptance claim. */
@RunWith(AndroidJUnit4::class)
class SelectionControllerInstrumentedTest {
    @Test fun completeNativeReceiptPrecedesOwnedProviderWrite() = runBlocking {
        fixture { controller, native, target, _ ->
            withContext(Dispatchers.Main) {
                assertTrue(controller.save(checkNotNull(controller.ticket(SelectionExportKind.Cutout, true)), testUri))
            }
            withTimeout(5000) { controller.saved.first { !it.saving && it.completed != null } }
            assertEquals(listOf("native", "provider"), native.events)
            assertArrayEquals(byteArrayOf(3, 7, 11), target.bytes)
            assertEquals(SelectionRegion(1u, 2u, 3u, 4u), native.exported!!.region)
            assertEquals(0, target.discarded)
        }
    }
    @Test fun cancelledNativeProducerSettlesBeforeStagingAndUriCleanup() = runBlocking {
        fixture { controller, native, target, root ->
            native.pause = CompletableDeferred(); native.settlement = CompletableDeferred()
            withContext(Dispatchers.Main) { controller.save(checkNotNull(controller.ticket(SelectionExportKind.Mask, false)), testUri) }
            withTimeout(5000) { native.entered.await() }
            withContext(Dispatchers.Main) { controller.cancel() }
            withTimeout(5000) { native.cancelled.await() }
            assertTrue(File(native.exported!!.outputPath).isFile); assertEquals(0, target.discarded)
            native.settlement!!.complete(Unit)
            withTimeout(5000) { controller.saved.first { !it.saving } }
            assertEquals(1, target.discarded); assertNull(target.bytes)
            assertEquals(0, File(root, "raster-transfers").listFiles()!!.size)
        }
    }
    @Test fun revisionChangeDuringProviderCopyDiscardsOnlyOwnedDestination() = runBlocking {
        fixture { controller, _, target, _ ->
            target.pause = CompletableDeferred()
            withContext(Dispatchers.Main) { controller.save(checkNotNull(controller.ticket(SelectionExportKind.Mask, false)), testUri) }
            withTimeout(5000) { target.entered.await() }
            withContext(Dispatchers.Main) { controller.observe(ProjectChange(2u, ChangeKind.Committed, testInfo.copy(stateHash = "pending"))) }
            target.pause!!.complete(Unit)
            withTimeout(5000) { controller.saved.first { !it.saving } }
            assertEquals(1, target.discarded); assertNull(controller.saved.value.completed)
        }
    }
    @Test fun tintUsesCoverageOnlyAndHasKnownAlphaEndpoints() {
        val result = selectionArgb(byteArrayOf(0, 128.toByte(), 255.toByte()))
        assertEquals(listOf(0, 55, 110), result.map { it ushr 24 })
        assertTrue(result.all { it and 0x00ffffff == 0x00d15bea })
    }
    private suspend fun fixture(block: suspend (SelectionController, MaskNativeFixture, DestinationFixture, File) -> Unit) = coroutineScope {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(context.cacheDir, "selection-test-${UUID.randomUUID()}")
        check(root.mkdir())
        val ownedContext = object : ContextWrapper(context) {
            override fun getApplicationContext(): Context = this
            override fun getCacheDir(): File = root
        }
        val owner = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val native = MaskNativeFixture(); val target = DestinationFixture(native.events)
        val controller = withContext(Dispatchers.Main) {
            SelectionController(ownedContext, owner, MaskCoreFixture(), {}, { native }, target).also {
                it.bind(MaskProjectFixture(), testDocument(), 1, 1u, Camera(Point(32.0, 32.0), 1.0, 0.0, 64.0, 64.0))
                it.interaction.useObject("mask")
            }
        }
        try {
            withTimeout(5000) { controller.state.first { it.selection != null && !it.loading } }
            block(controller, native, target, root)
        } finally {
            withContext(NonCancellable + Dispatchers.Main) { controller.detach() }; owner.cancel()
            val parent = File(root, "raster-transfers")
            if (parent.exists()) check(parent.delete()) { "Selection fixture staging did not settle" }
            check(root.delete()) { "Selection fixture still owns files" }
        }
    }
}
private val testUri: Uri = Uri.parse("content://visualworkbench.synthetic/selection")
private val testInfo = ProjectInfo("project", "Selection fixture", "device", 1u, false, false, 0u, "hash", listOf("doc"))
private fun testDocument() = DocumentSnapshot("doc", "Fixture", 64u, 64u, 8u, emptyList(), RenderList(testInfo, emptyList()))
private class DestinationFixture(private val events: MutableList<String>) : SelectionDestination {
    var pause: CompletableDeferred<Unit>? = null
    val entered = CompletableDeferred<Unit>()
    var bytes: ByteArray? = null
    var discarded = 0
    override suspend fun write(uri: Uri, source: File, expected: ULong, current: () -> Boolean) {
        assertEquals(testUri, uri); entered.complete(Unit); pause?.await()
        if (!current()) throw SelectionFailure(SelectionFailureKind.Conflict)
        assertEquals(expected, source.length().toULong()); events += "provider"; bytes = source.readBytes()
    }
    override suspend fun discard(uri: Uri): Boolean { assertEquals(testUri, uri); discarded++; return true }
}
private class MaskNativeFixture : WorkbenchSelections {
    val events = mutableListOf<String>()
    var pause: CompletableDeferred<Unit>? = null
    var settlement: CompletableDeferred<Unit>? = null
    val entered = CompletableDeferred<Unit>(); val cancelled = CompletableDeferred<Unit>()
    var exported: SelectionExportOptions? = null
    private val binding = SelectionBinding("project", "doc", "source", 0u, "hash")
    override suspend fun document(documentId: String) = SelectionDocument(binding, 64u, 64u, 8u, testInfo)
    override suspend fun snapshot(binding: SelectionBinding, objectId: String, memoryBudgetBytes: ULong) = SelectionSnapshot(binding,
        SelectionVersion(objectId, "coverage", 1u), "layer", 64u, 64u, SelectionRegion(1u, 2u, 3u, 4u), true, true)
    override suspend fun tiles(binding: SelectionBinding, selection: SelectionVersion, regions: List<SelectionRegion>, memoryBudgetBytes: ULong) =
        SelectionTiles(binding, selection, regions.map { SelectionTile(it, ByteArray((it.width * it.height).toInt())) })
    override suspend fun edit(options: SelectionEdit): SelectionReceipt = error("Unused")
    override suspend fun exportFile(options: SelectionExportOptions): SelectionExportReceipt {
        exported = options; File(options.outputPath).writeBytes(byteArrayOf(3, 7, 11)); entered.complete(Unit)
        try { pause?.await() }
        catch (error: CancellationException) { cancelled.complete(Unit); withContext(NonCancellable) { settlement?.await() }; throw error }
        events += "native"
        return SelectionExportReceipt(options.binding, options.selection, options.kind,
            options.region ?: SelectionRegion(0u, 0u, 64u, 64u), 8u, 3u, "0".repeat(64), "{}", testInfo, 1024u)
    }
}
private class MaskCoreFixture : WorkbenchCore {
    override fun newId(unixMs: ULong) = "id-$unixMs"
    override fun newDeviceId() = "device"
    override fun cameraMatrix(camera: Camera, inverse: Boolean) = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>) = points
    override suspend fun create(options: CreateProject): WorkbenchProject = error("Unused")
    override suspend fun open(path: String): WorkbenchProject = error("Unused")
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("Unused")
}
private class MaskProjectFixture : WorkbenchProject {
    override val changes = emptyFlow<ProjectChange>()
    override suspend fun info() = testInfo
    override suspend fun document(documentId: String) = testDocument()
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("Unused")
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("Unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Unused")
    override suspend fun render(documentId: String, rectangle: Rect?) = testDocument().render
    override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
    override suspend fun close() = Unit
}
