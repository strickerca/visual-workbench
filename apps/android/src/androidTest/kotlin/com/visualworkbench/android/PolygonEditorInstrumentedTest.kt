package com.visualworkbench.android

import android.app.Application
import android.graphics.Bitmap
import android.view.MotionEvent
import androidx.lifecycle.ViewModelStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.android.input.CanvasGestureRouter
import com.visualworkbench.android.input.PenSample
import com.visualworkbench.android.input.PointerFrame
import com.visualworkbench.android.input.PointerPacket
import com.visualworkbench.android.input.PointerTool
import com.visualworkbench.android.input.StylusCapabilities
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.io.File

/** Real router -> EditorController -> shared interaction -> native project.
 * Synthetic contacts only; this does not establish physical pen behavior. */
@RunWith(AndroidJUnit4::class)
class PolygonEditorInstrumentedTest {
    @Test fun separateDownUpContactsKeepVerticesAndFinishPublishesOneUndoableTransaction() = runBlocking {
        fixture { editor, router ->
            val before = withContext(Dispatchers.Main) { checkNotNull(editor.info).stateHash }
            withContext(Dispatchers.Main) {
                tap(editor, router, 30.0, 30.0, 100)
                tap(editor, router, 90.0, 30.0, 200)
                tap(editor, router, 60.0, 90.0, 300)
                assertEquals(3, editor.selections.state.value.draft?.points?.size)
                assertEquals(before, editor.info?.stateHash)
                assertTrue(editor.selections.interaction.finishPolygon())
                assertFalse(editor.selections.interaction.finishPolygon())
            }
            waitUntil { !editor.selections.state.value.busy && editor.selections.state.value.selection != null && editor.info?.stateHash != before }
            withContext(Dispatchers.Main) {
                assertEquals(1uL, editor.info?.hostSeq)
                assertNull(editor.selections.state.value.draft)
                editor.undo()
            }
            waitUntil { editor.pending == 0 && editor.info?.stateHash == before }
        }
    }

    @Test fun canceledContactAndChangedViewportCannotReuseEarlierPolygonVertices() = runBlocking {
        fixture { editor, router ->
            val before = withContext(Dispatchers.Main) { editor.info?.stateHash }
            withContext(Dispatchers.Main) {
                tap(editor, router, 30.0, 30.0, 100)
                tap(editor, router, 90.0, 30.0, 200)
                assertEquals(2, editor.selections.state.value.draft?.points?.size)
                assertInputReady(editor)
                router.accept(packet(MotionEvent.ACTION_DOWN, 60.0, 90.0, 300))
                router.accept(packet(MotionEvent.ACTION_CANCEL, 60.0, 90.0, 301, canceled = true))
                assertNull(editor.selections.state.value.draft)
                assertFalse(editor.selections.interaction.finishPolygon())
                tap(editor, router, 30.0, 30.0, 400)
                tap(editor, router, 90.0, 30.0, 500)
                assertEquals(2, editor.selections.state.value.draft?.points?.size)
                editor.pan(8.0, 0.0)
                assertNull(editor.selections.state.value.draft)
                assertFalse(editor.selections.interaction.finishPolygon())
            }
            waitUntil { inputReady(editor) }
            withContext(Dispatchers.Main) {
                tap(editor, router, 60.0, 90.0, 600)
                assertEquals(1, editor.selections.state.value.draft?.points?.size)
                assertFalse(editor.selections.interaction.finishPolygon())
                editor.cancelTool()
                assertNull(editor.selections.state.value.draft)
                assertEquals(before, editor.info?.stateHash)
            }
        }
    }

    private suspend fun fixture(block: suspend (EditorController, CanvasGestureRouter) -> Unit) {
        val app = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val actual = workbenchCore()
        val bitmap = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888)
        val source = try {
            bitmap.eraseColor(android.graphics.Color.WHITE)
            ByteArrayOutputStream().also { assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }.toByteArray()
        } finally { bitmap.recycle() }
        var ownedPath: File? = null
        val core = object : WorkbenchCore by actual {
            override suspend fun create(options: CreateProject): WorkbenchProject {
                ownedPath = File(options.path).canonicalFile
                return actual.create(options.copy(source = source))
            }
        }
        val store = ViewModelStore()
        val editor = withContext(Dispatchers.Main) { EditorController(app, core).also { store.put("polygon-contact", it) } }
        try {
            waitUntil { !editor.busy }
            withContext(Dispatchers.Main) { editor.viewport(128, 128, 1f); editor.newCanvas() }
            waitUntil { !editor.busy && editor.scene?.background != null }
            withContext(Dispatchers.Main) { editor.selections.interaction.tool(SelectionTool.Polygon) }
            // Selection canDraw alone does not include the editor's instruction,
            // semantic, document, modal and precision/display admission gates.
            waitUntil { inputReady(editor) }
            block(editor, CanvasGestureRouter(editor, 4.0))
        } finally {
            // Only the exact project created by this fixture is removed, and
            // only after ViewModel/native ownership has completely settled.
            withContext(NonCancellable) {
                withContext(Dispatchers.Main) { store.clear() }
                withTimeout(15_000) { checkNotNull(editor.scope.coroutineContext[Job]).join() }
                ownedPath?.let { owned ->
                    assertEquals(File(app.filesDir, "projects").canonicalFile, owned.parentFile)
                    assertTrue(owned.name.matches(Regex("[0-9a-fA-F-]{36}")))
                    assertTrue(!owned.exists() || owned.deleteRecursively())
                }
            }
        }
    }
    private suspend fun waitUntil(condition: () -> Boolean) = withTimeout(10_000) {
        while (!withContext(Dispatchers.Main) { condition() }) delay(10)
    }
    private fun inputReady(editor: EditorController): Boolean = editor.selections.state.value.canDraw &&
        editor.precisionInputAllowed() && editor.erasers.tools.route() == EraserInputRoute.Selection
    private fun assertInputReady(editor: EditorController) {
        assertTrue("Editor precision input must be admitted before the synthetic contact", editor.precisionInputAllowed())
        assertTrue("Polygon selection must be drawable before the synthetic contact", editor.selections.state.value.canDraw)
        assertEquals(EraserInputRoute.Selection, editor.erasers.tools.route())
    }
    private fun tap(editor: EditorController, router: CanvasGestureRouter, x: Double, y: Double, time: Long) {
        assertInputReady(editor)
        router.accept(packet(MotionEvent.ACTION_DOWN, x, y, time))
        router.accept(packet(MotionEvent.ACTION_UP, x, y, time + 1))
    }
    private fun packet(action: Int, x: Double, y: Double, time: Long, canceled: Boolean = false): PointerPacket {
        val sample = PenSample(1, PointerTool.Pen, x, y, time, .5f, .5f, null, null, 0)
        return PointerPacket(action, 1, canceled, time, listOf(PointerFrame(time, listOf(sample))),
            StylusCapabilities(true, false, false, false, false, false))
    }
}
