package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.Executors
import javax.imageio.ImageIO

/** Actual editor precision/pointer lifecycle -> shared selection -> native journal.
 * Synthetic host contacts, without a UI launch or physical pen claim. */
class PolygonEditorTest {
    @Test fun separateContactsRetainVerticesAndFinishIsExactlyOneUndoableTransaction() = fixture { editor ->
        val before = checkNotNull(editor.state.value.document).render.revision
        tap(editor, Point(25.0, 25.0), 100)
        tap(editor, Point(75.0, 25.0), 200)
        tap(editor, Point(50.0, 75.0), 300)
        assertEquals(3, editor.selections.state.value.draft?.points?.size)
        assertEquals(before, editor.state.value.document?.render?.revision)
        assertTrue(editor.selections.interaction.finishPolygon())
        assertFalse(editor.selections.interaction.finishPolygon())
        await { !editor.selections.state.value.busy && editor.state.value.document?.render?.revision?.hostSeq == before.hostSeq + 1u }
        assertNull(editor.selections.state.value.draft)
        editor.undo(false)
        await { editor.state.value.document?.render?.revision?.stateHash == before.stateHash }
    }

    @Test fun explicitCancelAndCameraChangesRetireIdlePolygonWithoutSaving() = fixture { editor ->
        val before = editor.state.value.document?.render?.revision
        tap(editor, Point(25.0, 25.0), 100)
        editor.cancelInput()
        assertNull(editor.selections.state.value.draft)
        tap(editor, Point(75.0, 25.0), 200)
        editor.pan(2.0, 0.0)
        assertNull(editor.selections.state.value.draft)
        assertFalse(editor.selections.interaction.finishPolygon())
        assertEquals(before, editor.state.value.document?.render?.revision)
    }

    private fun tap(editor: EditorController, point: Point, time: Long) {
        editor.precisionDown(point, false, false, time, .5f)
        editor.precisionUp(point, time + 1, .5f)
        // This is the same normal-UP cleanup as the actual Compose canvas.
        editor.finishPointerContact()
    }
    private suspend fun await(condition: () -> Boolean) = withTimeout(10_000) {
        while (!condition()) delay(5)
    }
    private fun fixture(block: suspend (EditorController) -> Unit) {
        Executors.newSingleThreadExecutor { r -> Thread(r, "vw-polygon-host-test").apply { isDaemon = true } }.asCoroutineDispatcher().use { dispatcher ->
            runBlocking(dispatcher) {
                val root = Files.createTempDirectory("vw-polygon-host-")
                val scope = CoroutineScope(coroutineContext + SupervisorJob())
                val core = workbenchCore(); val device = core.newDeviceId(); val time = System.currentTimeMillis()
                val pixels = BufferedImage(64, 64, BufferedImage.TYPE_INT_ARGB)
                val graphics = pixels.createGraphics()
                try { graphics.color = java.awt.Color.WHITE; graphics.fillRect(0, 0, 64, 64) } finally { graphics.dispose() }
                val source = ByteArrayOutputStream().also { check(ImageIO.write(pixels, "png", it)) }.toByteArray()
                val path = root.resolve("project")
                val created = core.create(CreateProject(path.toString(), core.newId(time.toULong()), core.newId(time.toULong()),
                    core.newId(time.toULong()), device, "Polygon fixture", source, time))
                created.close()
                val editor = EditorController(scope, core, object : DesktopHandoff {
                    override suspend fun paste(assumeSrgb: Boolean): ClipboardImage = error("Unused")
                    override suspend fun copy(path: Path, receipt: CompletedExport): Unit = error("Unused")
                    override suspend fun stage(path: Path, receipt: CompletedExport): DesktopDragFile = error("Unused")
                }, LocationPolicy(root, true, "owned synthetic fixture"), device)
                try {
                    editor.viewport(100.0, 100.0); editor.open(path)
                    await { !editor.state.value.busy && editor.state.value.background != null && !editor.instructionEditor.state.value.busy && !editor.semanticEditor.state.value.busy }
                    val state = editor.state.value
                    editor.canvasPrepared(checkNotNull(state.document).binding(state.projectEpoch))
                    editor.selections.interaction.tool(SelectionTool.Polygon)
                    withTimeout(5000) { editor.selections.state.first { it.canDraw } }
                    block(editor)
                } finally {
                    withContext(NonCancellable) { editor.close() }; scope.cancel()
                    Files.walk(root).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach { Files.delete(it) } }
                }
            }
        }
    }
}
