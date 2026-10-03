@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.awt.EventQueue
import java.awt.datatransfer.DataFlavor
import java.io.IOException
import java.nio.file.*
import java.nio.file.attribute.BasicFileAttributes
import java.util.concurrent.Executors

class DesktopTransferTest {
    @Test fun readinessBindsProjectAttachmentRevisionSettingsAndRegion() {
        val state = transferState()
        val full = captureExport(state, TransferCore(TransferProject()))
        val completed = completeExport(full, receipt(full))
        assertTrue(state.copy(exportReceipt = completed).exportReady)
        assertFalse(state.copy(exportReceipt = completed, projectEpoch = 9).exportReady)
        assertFalse(state.copy(exportReceipt = completed, document = transferDocument(8u)).exportReady)
        assertFalse(state.copy(exportReceipt = completed, exportSettings = state.exportSettings.copy(marked = false)).exportReady)
        assertTrue(state.copy(exportReceipt = completed, view = state.view.copy(camera = state.view.camera.copy(scale = 4.0))).exportReady)
        val selected = state.copy(exportSettings = state.exportSettings.copy(area = ExportArea.Selection), selected = setOf("object"))
        val selection = captureExport(selected, TransferCore(TransferProject()))
        assertEquals(Rect(1.0, 2.0, 4.0, 3.0), selection.region)
        assertFalse(selected.copy(exportReceipt = completeExport(selection, receipt(selection)), selected = emptySet()).exportReady)
        val viewed = state.copy(exportSettings = state.exportSettings.copy(area = ExportArea.View))
        val view = captureExport(viewed, TransferCore(TransferProject()))
        assertFalse(viewed.copy(exportReceipt = completeExport(view, receipt(view)), view = viewed.view.copy(camera = viewed.view.camera.copy(rotation = .2))).exportReady)
    }
    @Test fun cropBoundsUseOutwardPixelRoundingAndRefuseEmptyOrInvalidRegions() {
        assertEquals(Rect(0.0, 1.0, 4.0, 5.0), clippedExportRect(Rect(-1.5, 1.2, 5.3, 8.0), 8u, 6u))
        for (rect in listOf(Rect(100.0, 2.0, 1.0, 1.0), Rect(Double.NaN, 0.0, 2.0, 2.0), Rect(0.0, 0.0, -1.0, 1.0))) {
            assertThrows(HandoffFailure::class.java) { clippedExportRect(rect, 8u, 6u) }
        }
    }
    @Test fun completedReceiptMustMatchExactRevisionDimensionsAndCanonicalSource() {
        val request = captureExport(transferState(), TransferCore(TransferProject()))
        for (bad in listOf(receipt(request).copy(revision = transferInfo(8u)),
            receipt(request).copy(metadataJson = metadata(request).replace("\"output_width\":8", "\"output_width\":7")),
            receipt(request).copy(metadataJson = metadata(request).replace(SOURCE, "not-a-source")),
            receipt(request).copy(blake3 = "short"))) {
            assertThrows(HandoffFailure::class.java) { completeExport(request, bad) }
        }
        assertTrue(transferMessage(TransferFailure(TransferFailureKind.Alpha)).contains("matte"))
        assertTrue(transferMessage(TransferFailure(TransferFailureKind.Depth)).contains("16 bit"))
        assertTrue(transferMessage(TransferFailure(TransferFailureKind.PixelLimit)).contains("50 MP"))
    }
    @Test fun fileDragProvidesOnlyTheOneImmutableFileFlavor() {
        val file = Path.of("C:/synthetic/export.png")
        val transferable = PngFileTransferable(file)
        assertArrayEquals(arrayOf(DataFlavor.javaFileListFlavor), transferable.transferDataFlavors)
        assertFalse(transferable.isDataFlavorSupported(DataFlavor.stringFlavor))
        assertEquals(listOf(file.toFile()), transferable.getTransferData(DataFlavor.javaFileListFlavor))
    }
    @Test fun publicationIsCompleteNoClobberAndStalePreparationLeavesNoDestination() = temporary { root ->
        val bytes = ByteArray(180_001) { (it * 31).toByte() }
        val source = root.resolve("source"); Files.write(source, bytes)
        val target = root.resolve("out.png")
        publishExport(source, target, bytes.size.toULong()) { true }
        assertArrayEquals(bytes, Files.readAllBytes(target))
        try { publishExport(source, target, bytes.size.toULong()) { true }; fail("replaced existing destination") }
        catch (_: FileAlreadyExistsException) { }
        val stale = root.resolve("stale.png")
        try { publishExport(source, stale, bytes.size.toULong()) { false }; fail("published stale request") }
        catch (_: HandoffFailure) { }
        assertFalse(Files.exists(stale)); assertArrayEquals(bytes, Files.readAllBytes(source))
        val raced = root.resolve("raced.png")
        try {
            publishExport(source, raced, bytes.size.toULong()) {
                Files.writeString(raced, "concurrent destination", StandardOpenOption.CREATE_NEW); true
            }
            fail("overwrote a destination that appeared immediately before publication")
        } catch (_: FileAlreadyExistsException) { }
        assertEquals("concurrent destination", Files.readString(raced))
        Files.list(root).use { entries -> assertFalse(entries.anyMatch { it.fileName.toString().startsWith(".vw-export-") }) }
    }
    @Test fun privateStagingStreamsAndCleansPartialInputsWithoutTouchingOriginals() = temporary { root ->
        val source = root.resolve("source.png"); val bytes = ByteArray(131_073) { (it % 251).toByte() }; Files.write(source, bytes)
        val files = TransferFiles(root.resolve("private")); val workspace = files.create()
        try { assertArrayEquals(bytes, Files.readAllBytes(workspace.stage(source))); assertNotEquals(source, workspace.input) }
        finally { workspace.close() }
        assertFalse(Files.exists(workspace.directory)); assertArrayEquals(bytes, Files.readAllBytes(source))
        val refused = files.create()
        try {
            try { refused.stage(source, 100); fail("accepted over-limit import") } catch (_: HandoffFailure) { }
            try { withContext(Dispatchers.IO) { copyFile(source, refused.input, 100) }; fail("accepted an input that exceeded its streaming bound") }
            catch (_: HandoffFailure) { }
            assertTrue(Files.exists(refused.input))
        }
        finally { refused.close() }
        assertFalse(Files.exists(refused.directory))
        assertArrayEquals(bytes, Files.readAllBytes(source))
    }
    @Test fun nativeFileImportNeverUsesTheWholeBufferCreationApi() = controllerFixture { f ->
        val source = f.root.resolve("image.png"); Files.write(source, DATA)
        f.editor.importImage(source); f.idle { f.core.createdFiles == 1 }
        assertEquals(0, f.core.createdBuffers)
        assertArrayEquals(DATA, f.core.imported)
        assertNotEquals(source.toString(), f.core.lastSource)
        assertArrayEquals(DATA, Files.readAllBytes(source))
        assertFalse(Files.exists(Path.of(f.core.lastSource)))
    }
    @Test fun changedWorkspaceOwnershipRefusesCleanupAndPreservesItsFiles() = temporary { root ->
        val workspace = TransferFiles(root.resolve("private")).create()
        val marker = workspace.directory.resolve("owner")
        val token = Files.readString(marker)
        Files.write(workspace.input, DATA, StandardOpenOption.CREATE_NEW)
        Files.writeString(marker, "x".repeat(token.length), StandardOpenOption.TRUNCATE_EXISTING)
        try { workspace.close(); fail("deleted staging with changed ownership") }
        catch (_: HandoffFailure) { }
        assertArrayEquals(DATA, Files.readAllBytes(workspace.input))
        Files.writeString(marker, token, StandardOpenOption.TRUNCATE_EXISTING)
        workspace.close()
        assertFalse(Files.exists(workspace.directory))
    }
    @Test fun dropCompletesOnlyAfterDurablePrivateCopyAndCanReleaseOriginal() = controllerFixture { f ->
        val source = f.root.resolve("dropped.png"); Files.write(source, DATA)
        var completed = 0
        f.editor.acceptDrop(listOf(source)) { copied -> assertTrue(copied); completed++; Files.delete(source) }
        f.idle { f.core.createdFiles == 1 }
        assertEquals(1, completed); assertArrayEquals(DATA, f.core.imported); assertFalse(Files.exists(source))
    }
    @Test fun acceptedDropRetainsItsOnlyOriginalWhenNativeImportFails() = controllerFixture { f ->
        val source = f.root.resolve("rejected.png"); Files.write(source, DATA)
        f.core.importError = HandoffFailure("Synthetic decoder rejection")
        var completed = 0
        f.editor.acceptDrop(listOf(source)) { copied -> assertTrue(copied); completed++; Files.delete(source) }
        f.idle { f.editor.state.value.message?.contains("original bytes remain") == true }
        val recovery = Path.of(f.core.lastSource)
        assertEquals(1, completed); assertFalse(Files.exists(source))
        assertArrayEquals(DATA, Files.readAllBytes(recovery))
        val marker = recovery.parent.resolve("drop-recovery.txt")
        assertTrue(Files.size(marker) in 1..2048)
        assertTrue(Files.readString(marker).contains("rejected.png"))
        assertTrue(f.editor.state.value.message!!.contains(recovery.toString()))
    }
    @Test fun acceptedDropRetainsOriginalAfterDecoderCancellation() = controllerFixture { f ->
        val source = f.root.resolve("cancelled.png"); Files.write(source, DATA)
        val gate = ExportGate(); f.core.importGate = gate
        f.editor.acceptDrop(listOf(source)) { copied -> assertTrue(copied); Files.delete(source) }
        gate.started.await(); f.editor.cancelTransfer()
        f.idle { f.editor.state.value.message?.contains("original bytes remain") == true }
        assertFalse(Files.exists(source)); assertArrayEquals(DATA, Files.readAllBytes(Path.of(f.core.lastSource)))
    }
    @Test fun rejectedOrdinaryFileImportCleansStagingAndPreservesExternalOriginal() = controllerFixture { f ->
        val source = f.root.resolve("file.png"); Files.write(source, DATA)
        f.core.importError = HandoffFailure("Synthetic decoder rejection")
        f.editor.importImage(source); f.idle { f.core.createdFiles == 1 }
        assertArrayEquals(DATA, Files.readAllBytes(source))
        assertFalse(Files.exists(Path.of(f.core.lastSource)))
    }
    @Test fun nativeDropCallbackStaysOnStackWhileSecondaryLoopPumpsStagingCompletion() {
        val order = mutableListOf<String>()
        EventQueue.invokeAndWait {
            var callbackActive = true
            var nativeCompleted = false
            try {
                val copied = DropStagingWait(1000).await { complete ->
                    EventQueue.invokeLater {
                        assertTrue(callbackActive); order += "staged"
                        complete(true); complete(false) // late duplicate cannot change acceptance
                    }
                    val cancel: () -> Unit = { fail("cancelled completed staging") }
                    cancel
                }
                assertTrue(copied); nativeCompleted = true; order += "dropComplete"
            } finally {
                callbackActive = false
                // This is the native AWT contract: returning without completing
                // rejects the drag and may release its source temporary file.
                if (!nativeCompleted) order += "nativeFailure"
            }
            order += "returned"
        }
        assertEquals(listOf("staged", "dropComplete", "returned"), order)
    }
    @Test fun stagingWaitHandlesImmediateCompletionTimeoutDisposalAndLateCallbacks() {
        EventQueue.invokeAndWait {
            assertTrue(DropStagingWait(1000).await { complete -> complete(true); val cancel: () -> Unit = {}; cancel })
            var cancellations = 0
            lateinit var late: (Boolean) -> Unit
            val timed = DropStagingWait(25)
            assertFalse(timed.await { complete -> late = complete; val cancel: () -> Unit = { cancellations++ }; cancel })
            late(true); timed.close(); assertEquals(1, cancellations)
            val disposed = DropStagingWait(1000)
            assertFalse(disposed.await { complete ->
                EventQueue.invokeLater { disposed.close(); complete(true) }
                val cancel: () -> Unit = { cancellations++ }; cancel
            })
            assertEquals(2, cancellations)
        }
    }
    @Test fun stalePickerCannotImportIntoAnotherAttachment() = controllerFixture { f ->
        val source = f.root.resolve("old.png"); Files.write(source, DATA)
        val captured = ImportRequest(f.editor.state.value.projectEpoch, source)
        f.editor.closeProject(); f.idle { f.editor.state.value.document == null }
        f.editor.importFile(captured); f.idle { f.editor.state.value.message?.contains("project changed") == true }
        assertEquals(0, f.core.createdFiles); assertArrayEquals(DATA, Files.readAllBytes(source))
    }
    @Test fun changedSettingsDuringExportDiscardTheCompletedFileAndReadiness() = controllerFixture { f ->
        val request = f.editor.exportRequest()!!; val gate = ExportGate(); f.project.gate = gate
        val target = f.root.resolve("stale.png"); f.editor.saveExport(request, target)
        gate.started.await(); f.editor.exportSettings(request.settings.copy(marked = false)); gate.release.complete(Unit)
        f.idle { f.editor.state.value.message?.contains("changed") == true }
        assertFalse(Files.exists(target)); assertFalse(f.editor.state.value.exportReady)
        assertFalse(Files.exists(Path.of(f.project.lastOutput)))
    }
    @Test fun nativeReceiptForAnotherRevisionCannotPublishOrEnableReady() = controllerFixture { f ->
        f.project.returnAnotherRevision = true
        f.editor.prepareExportFile(f.editor.exportRequest()!!)
        f.idle { f.project.exports == 1 }
        assertFalse(f.editor.state.value.exportReady); assertFalse(Files.exists(Path.of(f.project.lastOutput)))
    }
    @Test fun cancelledExportSettlesBeforePrivateWorkspaceCleanup() = controllerFixture { f ->
        val gate = ExportGate(); f.project.gate = gate
        f.editor.copyExport(f.editor.exportRequest()!!)
        gate.started.await(); f.editor.cancelTransfer()
        f.idle { f.editor.state.value.message?.contains("Cancellation settled") == true }
        assertFalse(Files.exists(Path.of(f.project.lastOutput)))
        assertFalse(f.editor.state.value.exportReady); assertEquals(0, f.handoff.copies)
    }
    @Test fun clipboardCopyUsesTheCompletedFileAndNeverBackgroundPixels() = controllerFixture { f ->
        val request = f.editor.exportRequest()!!
        f.editor.copyExport(request); f.idle { f.handoff.copies == 1 }
        assertArrayEquals(DATA, f.handoff.copied)
        assertEquals(request, f.handoff.lastReceipt?.request); assertTrue(f.editor.state.value.exportReady)
        assertEquals(0, f.project.bufferedExports)
    }
    @Test fun preparedDragInvalidatesOnSettingsChangeAndGestureOwnsTransferredLease() = controllerFixture { f ->
        f.editor.prepareDragExport(f.editor.exportRequest()!!); f.idle { f.editor.state.value.dragPrepared }
        val first = f.handoff.leases.single(); f.editor.exportSettings(f.editor.state.value.exportSettings.copy(marked = false))
        assertEquals(1, first.closed); assertFalse(f.editor.state.value.dragPrepared)
        f.editor.prepareDragExport(f.editor.exportRequest()!!); f.idle { f.editor.state.value.dragPrepared }
        val second = f.editor.takePreparedDrag()!!; assertFalse(f.editor.state.value.dragPrepared)
        f.editor.exportSettings(f.editor.state.value.exportSettings.copy(marked = true))
        assertEquals(0, f.handoff.leases.last().closed)
        second.close(); assertEquals(1, f.handoff.leases.last().closed)
    }
    @Test fun mp4AttachesToCurrentProjectWithoutCreatingTimelineOrNewDocument() = controllerFixture { f ->
        val source = f.root.resolve("generated.mp4"); Files.write(source, DATA)
        f.editor.acceptDrop(listOf(source)); f.idle { f.project.attachments == 1 }
        assertEquals(0, f.core.createdFiles); assertEquals(FileAssetKind.Mp4, f.project.attachedKind)
        assertEquals("document", f.editor.state.value.document?.documentId)
        assertArrayEquals(DATA, Files.readAllBytes(source))
    }
    @Test fun explicitPasteAloneReadsTheInjectedClipboard() = controllerFixture { f ->
        assertEquals(0, f.handoff.reads)
        f.editor.showPaste(); f.editor.dismissPaste(); assertEquals(0, f.handoff.reads)
        f.editor.pasteImage(true); f.idle { f.core.createdFiles == 1 }
        assertEquals(1, f.handoff.reads); assertTrue(f.handoff.assumed)
        assertArrayEquals(DATA, f.core.imported)
    }
}

private val DATA = ByteArray(129) { (it * 17).toByte() }
private val HASH = "a".repeat(64)
private val SOURCE = "b".repeat(64)
private fun transferInfo(sequence: ULong = 7u) = ProjectInfo("project", "fixture", "owner", sequence + 1u, true, false, sequence, sequence.toString(16).padStart(64, '0'), listOf("document"))
private fun transferDocument(sequence: ULong = 7u): DocumentSnapshot = DocumentSnapshot("document", "fixture", 8u, 6u, 8u,
    listOf(LayerInfo("layer", "Annotations", true, false, 1.0, "normal")), RenderList(transferInfo(sequence), listOf(
        RenderItem("object", "layer", 1.0, "normal", Rect(1.2, 2.1, 3.1, 2.1), Contours(longArrayOf(), longArrayOf(), uintArrayOf()),
            Transform(), ObjectStyle(0xffffffffu, 1.0), Shape.Rectangle(Rect(1.2, 2.1, 3.1, 2.1)), false))))
private fun transferState() = EditorState(document = transferDocument(), projectEpoch = 1,
    view = ViewState(Camera(Point(4.0, 3.0), 1.0, 0.0, 8.0, 6.0)))
private fun metadata(request: ExportRequest) = """{"revision":"${request.revision}","source_asset":"$SOURCE","output_width":${request.width},"output_height":${request.height}}"""
private fun receipt(request: ExportRequest) = FileExportResult(DATA.size.toULong(), HASH, metadata(request), transferInfo(request.hostSeq), 1024u)
private class ExportGate { val started = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>() }
private class TransferProject : WorkbenchProject, WorkbenchFileTransfers {
    var current = transferDocument(); var exports = 0; var attachments = 0; var bufferedExports = 0
    var gate: ExportGate? = null; var returnAnotherRevision = false; var lastOutput = ""; var attachedKind: FileAssetKind? = null
    override val changes = MutableSharedFlow<ProjectChange>(extraBufferCapacity = 8)
    override suspend fun info() = current.render.revision
    override suspend fun document(documentId: String) = current
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean) =
        BackgroundImage(8u, 6u, ByteArray(8 * 6 * 4) { 255.toByte() }, SOURCE, 8u, byteArrayOf())
    override suspend fun preflightFile(options: FileExportOptions): FilePreflight = FilePreflight(
        options.export.region?.width?.toUInt() ?: 8u, options.export.region?.height?.toUInt() ?: 6u, 8u,
        if (options.export.format == ImageFormat.Png16) 16u else 8u, 1024u, false, options.export.marked, info())
    override suspend fun exportTransfer(options: FileExportOptions): FileExportResult {
        exports++; lastOutput = options.outputPath
        gate?.also { it.started.complete(Unit); it.release.await(); gate = null }
        Files.write(Path.of(options.outputPath), DATA, StandardOpenOption.CREATE_NEW)
        val request = ExportRequest(1, "project", "document", current.render.revision.hostSeq, current.render.revision.stateHash,
            ExportSettings(), options.export.region, options.export.region?.width?.toUInt() ?: 8u, options.export.region?.height?.toUInt() ?: 6u, emptySet(), null)
        return receipt(request).copy(revision = if (returnAnotherRevision) transferInfo(8u) else info())
    }
    override suspend fun attachFile(options: AttachFileOptions): AttachedAsset {
        assertArrayEquals(DATA, Files.readAllBytes(Path.of(options.sourcePath)))
        attachments++; attachedKind = options.kind; current = transferDocument(current.render.revision.hostSeq + 1u)
        return AttachedAsset(HASH, DATA.size.toULong(), "mp4", info())
    }
    override suspend fun export(options: ExportOptions): ExportResult { bufferedExports++; error("whole-buffer export forbidden") }
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
    override suspend fun render(documentId: String, rectangle: Rect?) = current.render
    override suspend fun close() = Unit
}
private class TransferCore(private val project: TransferProject) : WorkbenchCore, WorkbenchStreamingCore {
    var serial = 0; var createdFiles = 0; var createdBuffers = 0; var imported = byteArrayOf(); var lastSource = ""
    var importError: Exception? = null; var importGate: ExportGate? = null
    override fun newId(unixMs: ULong) = "id-${serial++}"
    override fun newDeviceId() = "owner"
    override suspend fun create(options: CreateProject): WorkbenchProject { createdBuffers++; error("whole-buffer import forbidden") }
    override suspend fun createFile(options: CreateFileProject): WorkbenchProject {
        createdFiles++; lastSource = options.sourcePath; imported = Files.readAllBytes(Path.of(options.sourcePath))
        importGate?.also { it.started.complete(Unit); it.release.await() }
        importError?.let { throw it }
        return project
    }
    override suspend fun open(path: String): WorkbenchProject = project
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("unused")
    override fun cameraMatrix(camera: Camera, inverse: Boolean) = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>) = points
}
private class FakeDrag(override val path: Path) : DesktopDragFile { var closed = 0; override fun close() { closed++ } }
private class TransferHandoff : DesktopHandoff {
    var copies = 0; var reads = 0; var assumed = false; var copied = byteArrayOf(); var lastReceipt: CompletedExport? = null
    val leases = mutableListOf<FakeDrag>()
    override suspend fun paste(assumeSrgb: Boolean): ClipboardImage { reads++; assumed = assumeSrgb; return ClipboardImage(DATA.copyOf()) }
    override suspend fun copy(path: Path, receipt: CompletedExport) { copied = Files.readAllBytes(path); lastReceipt = receipt; copies++ }
    override suspend fun stage(path: Path, receipt: CompletedExport): DesktopDragFile = FakeDrag(path).also { leases += it }
}
private class TransferFixture(val root: Path, scope: CoroutineScope) {
    val project = TransferProject(); val core = TransferCore(project); val handoff = TransferHandoff()
    val editor = EditorController(scope, core, handoff, LocationPolicy(root.resolve("projects"), true, "fixture"), "owner", root.resolve("private"))
    suspend fun idle(condition: () -> Boolean) { withTimeout(5000) { editor.state.first { !it.busy && condition() } }; yield() }
}
private fun controllerFixture(block: suspend (TransferFixture) -> Unit) = temporary { root ->
    Executors.newSingleThreadExecutor().asCoroutineDispatcher().use { dispatcher ->
        withContext(dispatcher) {
            val scope = CoroutineScope(SupervisorJob() + dispatcher); val fixture = TransferFixture(root, scope)
            try {
                fixture.editor.open(root.resolve("project")); fixture.idle { fixture.editor.state.value.document != null }
                block(fixture)
            } finally { fixture.editor.close(); scope.cancel() }
        }
    }
}
private fun temporary(block: suspend (Path) -> Unit) = runBlocking {
    val root = Files.createTempDirectory("vw-desktop-transfer-").toRealPath()
    try { withTimeout(15_000) { block(root) } }
    finally {
        check(root.fileName.toString().startsWith("vw-desktop-transfer-") && root.toRealPath() == root)
        Files.walkFileTree(root, object : SimpleFileVisitor<Path>() {
            override fun visitFile(file: Path, attributes: BasicFileAttributes): FileVisitResult { check(file.startsWith(root)); Files.delete(file); return FileVisitResult.CONTINUE }
            override fun postVisitDirectory(dir: Path, error: IOException?): FileVisitResult { if (error != null) throw error; check(dir.startsWith(root)); Files.delete(dir); return FileVisitResult.CONTINUE }
        })
    }
}
