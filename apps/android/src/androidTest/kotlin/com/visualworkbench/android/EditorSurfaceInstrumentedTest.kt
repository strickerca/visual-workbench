package com.visualworkbench.android

import android.os.SystemClock
import android.util.Log
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.*
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.Shape
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/** Actual app SurfaceView MotionEvents + native persistence; synthetic input only. */
@RunWith(AndroidJUnit4::class)
class EditorSurfaceInstrumentedTest {
    private fun ready(activity: MainActivity): Boolean = !activity.editor.busy && activity.editor.pending == 0 &&
        !activity.editor.instructionEditor.state.value.busy && !activity.editor.semanticEditor.state.value.busy

    private fun await(scenario: ActivityScenario<MainActivity>, phase: String, predicate: (MainActivity) -> Boolean) {
        val deadline = SystemClock.uptimeMillis() + 45_000
        var nextProgress = SystemClock.uptimeMillis() + 5_000
        while (SystemClock.uptimeMillis() < deadline) {
            var ready = false
            scenario.onActivity { ready = predicate(it) }
            if (ready) return
            if (SystemClock.uptimeMillis() >= nextProgress) {
                Log.i("VW_EDITOR_TEST", "phase=$phase waiting=true"); nextProgress += 5_000
            }
            SystemClock.sleep(25)
        }
        var detail = ""
        scenario.onActivity { detail = "busy=${it.editor.busy} pending=${it.editor.pending} message=${it.editor.message} blocked=${it.editor.blockingMessage}" }
        fail("Timed out in $phase: $detail")
    }
    private fun canvas(view: View): CanvasSurface? {
        if (view is CanvasSurface) return view
        if (view is ViewGroup) for (i in 0 until view.childCount) canvas(view.getChildAt(i))?.let { return it }
        return null
    }
    private fun gesture(scenario: ActivityScenario<MainActivity>, tool: EditorTool, points: List<Point>, cancel: Boolean = false, finger: Boolean = false) {
        // The real input gate also waits for the instruction/semantic refreshes
        // that share the native project worker after an edit.
        await(scenario, "input-ready-$tool", ::ready)
        scenario.onActivity { activity ->
            val editor = activity.editor; editor.chooseTool(tool)
            val surface = requireNotNull(canvas(activity.window.decorView))
            val snapshot = requireNotNull(editor.scene)
            val screen = points.map { EditorDrawing.map(snapshot.matrix, it) }
            val start = SystemClock.uptimeMillis()
            screen.forEachIndexed { index, p ->
                val action = when (index) { 0 -> MotionEvent.ACTION_DOWN; screen.lastIndex -> if (cancel) MotionEvent.ACTION_CANCEL else MotionEvent.ACTION_UP; else -> MotionEvent.ACTION_MOVE }
                val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = if (finger) MotionEvent.TOOL_TYPE_FINGER else MotionEvent.TOOL_TYPE_STYLUS })
                val coordinates = arrayOf(MotionEvent.PointerCoords().apply { x = p.x.toFloat(); y = p.y.toFloat(); pressure = .2f + .7f * index / maxOf(1, screen.lastIndex); size = .01f })
                val event = MotionEvent.obtain(start, start + index * 8, action, 1, properties, coordinates, 0, 0, 1f, 1f, -1, 0,
                    if (finger) InputDevice.SOURCE_TOUCHSCREEN else InputDevice.SOURCE_STYLUS, 0)
                try { assertTrue(surface.dispatchTouchEvent(event)) } finally { event.recycle() }
            }
        }
        await(scenario, "gesture-$tool", ::ready)
    }
    private fun withCanvas(body: (ActivityScenario<MainActivity>, String) -> Unit) {
        var ownedProject: String? = null
        var storageClosed = false
        try {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            try {
                await(scenario, "startup") { !it.editor.busy }
                scenario.onActivity { it.editor.newCanvas() }
                await(scenario, "new-project-and-background") { ready(it) && it.editor.scene?.background != null && canvas(it.window.decorView)?.holder?.surface?.isValid == true }
                scenario.onActivity { ownedProject = requireNotNull(it.editor.info).projectId; assertNull(it.editor.blockingMessage) }
                body(scenario, requireNotNull(ownedProject))
            } finally {
                scenario.onActivity { it.editor.cancelTool() }
                await(scenario, "finish-owned-edits") { it.editor.pending == 0 }
                scenario.onActivity { it.editor.navigate(WorkbenchScreen.Projects) }
                await(scenario, "close-owned-project") { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
                storageClosed = true
            }
        }
        } finally { ownedProject?.let { id ->
            val root = File(InstrumentationRegistry.getInstrumentation().targetContext.filesDir, "projects").canonicalFile
            val target = File(root, id).canonicalFile
            assertEquals(root, target.parentFile); assertTrue(id.matches(Regex("[0-9a-fA-F-]{36}")))
            if (storageClosed) {
                assertTrue(!target.exists() || target.deleteRecursively())
                Log.i("VW_EDITOR_TEST", "owned_project_cleanup=true")
            } else Log.e("VW_EDITOR_TEST", "owned_project_cleanup=false reason=storage_close_unconfirmed")
        } }
    }

    @Test fun everyDrawingToolPersistsAndCancellationCreatesNoObject() = withCanvas { scenario, id ->
        val drawingTools = listOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker, EditorTool.Line, EditorTool.Arrow, EditorTool.Rectangle, EditorTool.Ellipse)
        for ((i, tool) in drawingTools.withIndex()) {
            val y = 100.0 + i * 115
            gesture(scenario, tool, listOf(Point(260.0, y), Point(330.0, y + 15), Point(420.0, y + 45)))
            scenario.onActivity { assertEquals("tool=$tool ${it.editor.message}", i + 1, it.editor.document?.render?.items?.size) }
        }
        gesture(scenario, EditorTool.Pen, listOf(Point(600.0, 200.0), Point(720.0, 210.0), Point(800.0, 230.0)), cancel = true)
        scenario.onActivity { assertEquals(7, it.editor.document?.render?.items?.size) }
        // A finger uses navigation by default, even while Pen is selected.
        gesture(scenario, EditorTool.Pen, listOf(Point(650.0, 300.0), Point(700.0, 300.0), Point(750.0, 300.0)), finger = true)
        scenario.onActivity { assertEquals(7, it.editor.document?.render?.items?.size); it.editor.fit() }
        var hash = ""
        scenario.onActivity { hash = requireNotNull(it.editor.info).stateHash; it.editor.navigate(WorkbenchScreen.Projects) }
        await(scenario, "closed-for-reopen") { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
        scenario.onActivity { it.editor.open(id) }
        await(scenario, "reopen-document") { !it.editor.busy && it.editor.document != null }
        scenario.onActivity {
            assertEquals(hash, it.editor.info?.stateHash)
            assertEquals(7, it.editor.document?.render?.items?.size)
            assertTrue(it.editor.document?.render?.items?.any { item -> item.layerBlend == "multiply" } == true)
        }
    }

    @Test fun textSelectionMoveResizeEraserUndoAndActivityRecreationUseRealCore() = withCanvas { scenario, _ ->
        gesture(scenario, EditorTool.Rectangle, listOf(Point(420.0, 260.0), Point(600.0, 420.0)))
        val created = arrayOf("")
        scenario.onActivity { created[0] = requireNotNull(it.editor.document).render.items.single().objectId }
        gesture(scenario, EditorTool.Select, listOf(Point(420.0, 300.0), Point(460.0, 330.0)))
        scenario.onActivity {
            val item = requireNotNull(it.editor.document).render.items.single()
            assertEquals(created[0], item.objectId); assertEquals(40.0, item.transform.e, .02); assertEquals(30.0, item.transform.f, .02)
        }
        var corner = Point(0.0, 0.0)
        scenario.onActivity { val b = requireNotNull(it.editor.document).render.items.single().bounds; corner = Point(b.x + b.width, b.y + b.height) }
        gesture(scenario, EditorTool.Select, listOf(corner, Point(corner.x + 80, corner.y + 60)))
        scenario.onActivity { assertTrue(requireNotNull(it.editor.document).render.items.single().transform.a > 1) }
        gesture(scenario, EditorTool.Text, listOf(Point(500.0, 620.0), Point(500.0, 620.0)))
        scenario.onActivity { assertNotNull(it.editor.textDraft); it.editor.saveText("Editable text", "Inter", 36.0) }
        await(scenario, "save-text") { !it.editor.busy && it.editor.pending == 0 && it.editor.textDraft == null }
        scenario.onActivity { assertTrue(it.editor.document?.render?.items?.any { item -> item.shape is Shape.Text } == true) }
        gesture(scenario, EditorTool.Eraser, listOf(Point(455.0, 290.0), Point(475.0, 360.0)), cancel = true)
        scenario.onActivity { assertEquals(2, it.editor.document?.render?.items?.size) }
        // Explicit selected-object deletion is one transaction; undo restores it.
        scenario.onActivity { it.editor.deleteSelected() }
        await(scenario, "delete-selected") { it.editor.pending == 0 }
        scenario.onActivity { assertEquals(1, it.editor.document?.render?.items?.size); it.editor.undo() }
        await(scenario, "undo-delete") { it.editor.pending == 0 }
        var hash = ""
        scenario.onActivity { assertEquals(2, it.editor.document?.render?.items?.size); hash = requireNotNull(it.editor.info).stateHash }
        scenario.recreate()
        await(scenario, "renderer-after-recreation") { it.editor.scene?.background != null && canvas(it.window.decorView)?.holder?.surface?.isValid == true }
        scenario.onActivity { assertEquals(hash, it.editor.info?.stateHash); assertEquals(2, it.editor.document?.render?.items?.size) }
        gesture(scenario, EditorTool.Pen, listOf(Point(900.0, 600.0), Point(970.0, 650.0)))
        scenario.onActivity { assertEquals(3, it.editor.document?.render?.items?.size) }
    }
}
