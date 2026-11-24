@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.CanvasSurface
import com.visualworkbench.android.editor.EditorDrawing
import com.visualworkbench.android.editor.EditorTool
import com.visualworkbench.android.editor.WorkbenchScreen
import com.visualworkbench.shared.Contours
import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.EditorWorkflowPanel
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.RenderList
import com.visualworkbench.shared.SampleBatch
import com.visualworkbench.shared.Shape
import com.visualworkbench.shared.StrokeOptions
import com.visualworkbench.shared.WorkflowBinding
import com.visualworkbench.shared.workbenchCore
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/** Many pen strokes drawn faster than the editor finishes with each one must
 * all be saved. The real app, its SurfaceView's own touch dispatch and a real
 * native project; the strokes are counted in the project itself after it has
 * been closed. Synthetic pen events only: this proves nothing about a pen's
 * hardware, pressure or feel.
 *
 * Every test also checks what the editor was left with once saving had
 * settled: the document it held must be, item for item and at the same
 * revision, the document a read of the project returns, and its layer
 * controls must load for that document.
 *
 * The two tests that count saved strokes use nothing but what the app has
 * always had, so this file can be run unchanged against an older build to
 * measure it. The third reads the refused pen-down count by name and fails on
 * a build that does not keep one. */
@RunWith(AndroidJUnit4::class)
class PenStrokeBurstInstrumentedTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private val core by lazy { workbenchCore() }

    private fun await(scenario: ActivityScenario<MainActivity>, phase: String, timeoutMs: Long = 45_000, predicate: (MainActivity) -> Boolean) {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            var ready = false
            scenario.onActivity { ready = predicate(it) }
            if (ready) return
            SystemClock.sleep(25)
        }
        var detail = ""
        scenario.onActivity { activity ->
            val editor = activity.editor
            detail = "busy=${editor.busy} pending=${editor.pending} message=${editor.message} blocked=${editor.blockingMessage} " +
                "screen=${editor.screen} scene=${editor.scene != null} background=${editor.scene?.background != null}"
        }
        fail("Timed out in $phase: $detail")
    }
    private fun canvas(view: View): CanvasSurface? {
        if (view is CanvasSurface) return view
        if (view is ViewGroup) for (i in 0 until view.childCount) canvas(view.getChildAt(i))?.let { return it }
        return null
    }
    private fun canvasReady(activity: MainActivity): Boolean {
        val editor = activity.editor
        return !editor.busy && editor.pending == 0 && !editor.instructionEditor.state.value.busy && !editor.semanticEditor.state.value.busy &&
            editor.scene?.background != null && canvas(activity.window.decorView)?.holder?.surface?.isValid == true
    }
    private fun projectPath(id: String): String {
        require(id.matches(Regex("[0-9a-fA-F-]{36}")))
        return File(File(context.filesDir, "projects"), id).canonicalPath
    }

    /** The document the closed project holds, read from the core. */
    private fun savedDocument(id: String): DocumentSnapshot = runBlocking {
        val project = core.open(projectPath(id))
        try { project.document(project.info().documentIds.first()) }
        finally { project.close() }
    }

    /** Null when [held], the document the editor showed, is the document
     * [read] from the project: the same revision, frame and layers, and the
     * same items in the same drawing order, outlines included. Otherwise the
     * first difference. */
    private fun difference(held: DocumentSnapshot?, read: DocumentSnapshot): String? {
        if (held == null) return "the editor held no document"
        val a = held.render.revision; val b = read.render.revision
        if (a.projectId != b.projectId || a.hostSeq != b.hostSeq || a.stateHash != b.stateHash)
            return "held revision ${a.hostSeq}/${a.stateHash}, the project is at ${b.hostSeq}/${b.stateHash}"
        val frame = RenderList(a, emptyList())
        if (held.copy(render = frame) != read.copy(render = frame)) return "the title, size or layers differ"
        val heldIds = held.render.items.map { it.objectId }; val readIds = read.render.items.map { it.objectId }
        if (heldIds != readIds) {
            val at = heldIds.zip(readIds).indexOfFirst { (x, y) -> x != y }.let { if (it < 0) minOf(heldIds.size, readIds.size) else it }
            return "held ${heldIds.size} items, a read lists ${readIds.size}; they differ from place $at on"
        }
        val empty = Contours(longArrayOf(), longArrayOf(), uintArrayOf())
        for ((index, pair) in held.render.items.zip(read.render.items).withIndex()) {
            val (x, y) = pair
            if (x.copy(contours = empty) != y.copy(contours = empty) || !x.contours.x.contentEquals(y.contours.x) ||
                !x.contours.y.contentEquals(y.contours.y) || !x.contours.ends.contentEquals(y.contours.ends))
                return "item $index (${x.objectId}) is not the item a read returns"
        }
        return null
    }

    /** Opens the layer controls and waits for the list of the document that is
     * shown. Null when it loads; otherwise what was seen instead. Controls
     * left waiting for a document that never comes keep loading. */
    private fun layerControlsIssue(scenario: ActivityScenario<MainActivity>): String? {
        val deadline = SystemClock.uptimeMillis() + 30_000
        var seen = "the layer controls could not be opened"
        var bound = false
        while (!bound && SystemClock.uptimeMillis() < deadline) {
            scenario.onActivity { activity ->
                val editor = activity.editor; val workflows = editor.workflows
                if (workflows.panel.value != EditorWorkflowPanel.Layers) workflows.activate(EditorWorkflowPanel.Layers, true)
                if (workflows.panel.value == EditorWorkflowPanel.Layers) {
                    val state = workflows.controls.state.value
                    val shown = editor.document; val revision = shown?.render?.revision
                    val expected = if (shown == null || revision == null) null
                        else WorkflowBinding(revision.projectId, shown.documentId, revision.hostSeq, revision.stateHash)
                    bound = !state.loading && state.snapshot != null && state.snapshot?.binding == expected
                    seen = "loading=${state.loading} listed=${state.snapshot != null} for_the_document_shown=${state.snapshot?.binding == expected} issue=${state.issue}"
                } else seen = "the layer controls did not open: ${editor.message}"
            }
            if (!bound) SystemClock.sleep(25)
        }
        scenario.onActivity { it.editor.workflows.activate(EditorWorkflowPanel.Layers, false) }
        return if (bound) null else seen
    }

    /** Puts [count] short strokes into the closed project through the core,
     * so a full page does not have to be drawn through the screen first. */
    private fun fillPage(id: String, count: Int): Unit = runBlocking {
        val project = core.open(projectPath(id))
        try {
            var info = project.info()
            val documentId = info.documentIds.first()
            val layer = project.document(documentId).layers.first { it.visible && !it.locked && it.blend == "normal" }
            repeat(count) { index ->
                val now = System.currentTimeMillis()
                val stroke = project.beginStroke(StrokeOptions(core.newId(now.toULong()), core.newId(now.toULong()), core.newId(now.toULong()),
                    documentId, layer.id, info.deviceId, info.nextLamport, now, "pen", 3.0, 0x1b2a3affu))
                try {
                    val x = 60.0 + (index % 60) * 24.0; val y = 660.0 + (index / 60) * 22.0
                    stroke.append(SampleBatch(1uL, DoubleArray(8) { x + it * 2.5 }, DoubleArray(8) { y + (it % 3) * 3.0 },
                        UIntArray(8) { (it * 8).toUInt() }, FloatArray(8) { .3f + it * .08f }))
                    info = stroke.commit()
                } finally { stroke.dispose() }
            }
        } finally { project.close() }
    }

    /** [count] pen strokes, each [strokeMs] long with [gapMs] between the
     * lift and the next contact, through the canvas view's touch dispatch.
     * Every event is posted for its own moment before the first is due, as a
     * pen delivers them: nothing here waits for the editor to be ready for
     * the next stroke. Returns once the last event has been delivered. */
    private fun drawStrokes(scenario: ActivityScenario<MainActivity>, count: Int, strokeMs: Long, gapMs: Long) {
        val delivered = CountDownLatch(1)
        val failure = AtomicReference<Throwable?>(null)
        val steps = (strokeMs / 8).toInt().coerceAtLeast(2)
        scenario.onActivity { activity ->
            val editor = activity.editor; editor.chooseTool(EditorTool.Pen)
            val surface = requireNotNull(canvas(activity.window.decorView))
            val matrix = requireNotNull(editor.scene).matrix
            val main = Handler(Looper.getMainLooper())
            val first = SystemClock.uptimeMillis() + 500
            for (stroke in 0 until count) {
                // Short marks on a grid across the upper part of the page.
                val from = Point(80.0 + (stroke % 40) * 36.0, 60.0 + (stroke / 40 % 14) * 40.0)
                val down = first + stroke * (strokeMs + gapMs)
                for (step in 0..steps) {
                    val time = down + strokeMs * step / steps
                    val screen = EditorDrawing.map(matrix, Point(from.x + 22.0 * step / steps, from.y + 16.0 * step / steps))
                    val action = when (step) { 0 -> MotionEvent.ACTION_DOWN; steps -> MotionEvent.ACTION_UP; else -> MotionEvent.ACTION_MOVE }
                    main.postAtTime({
                        if (failure.get() == null) try {
                            val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_STYLUS })
                            val coordinates = arrayOf(MotionEvent.PointerCoords().apply {
                                x = screen.x.toFloat(); y = screen.y.toFloat(); pressure = .25f + .6f * step / steps; size = .01f })
                            val event = MotionEvent.obtain(down, time, action, 1, properties, coordinates, 0, 0, 1f, 1f, -1, 0, InputDevice.SOURCE_STYLUS, 0)
                            try { check(surface.dispatchTouchEvent(event)) { "The canvas did not take the pen event" } } finally { event.recycle() }
                        } catch (error: Throwable) { failure.compareAndSet(null, error) }
                        if (stroke == count - 1 && step == steps) delivered.countDown()
                    }, time)
                }
            }
        }
        val allowed = count * (strokeMs + gapMs) + 120_000
        assertTrue("The pen events were not all delivered within $allowed ms", delivered.await(allowed, TimeUnit.MILLISECONDS))
        failure.get()?.let { throw AssertionError("A pen event could not be delivered", it) }
    }

    /** A new blank page in the real app, [filled] strokes put into it through
     * the core, then [body] with the page open and [settled] once every
     * stroke is saved; afterwards the page is closed and [closed] gets the
     * id of the project. The project is deleted at the end. */
    private fun withPage(filled: Int, body: (ActivityScenario<MainActivity>) -> Unit, settled: (ActivityScenario<MainActivity>) -> Unit, closed: (String) -> Unit) {
        var owned: String? = null
        var storageClosed = false
        try {
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                await(scenario, "startup") { !it.editor.busy }
                scenario.onActivity { it.editor.newCanvas() }
                await(scenario, "new-page", predicate = ::canvasReady)
                scenario.onActivity { owned = requireNotNull(it.editor.info).projectId; assertNull(it.editor.blockingMessage) }
                val id = requireNotNull(owned)
                try {
                    if (filled > 0) {
                        scenario.onActivity { it.editor.navigate(WorkbenchScreen.Projects) }
                        await(scenario, "closed-to-fill") { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
                        fillPage(id, filled)
                        scenario.onActivity { it.editor.open(id) }
                        await(scenario, "filled-page", timeoutMs = 120_000, predicate = ::canvasReady)
                        scenario.onActivity { assertEquals(filled, it.editor.document?.render?.items?.size) }
                    }
                    body(scenario)
                    // Every stroke taken is saved before the page can close.
                    await(scenario, "strokes-saved", timeoutMs = 180_000) { it.editor.pending == 0 && !it.editor.busy }
                    settled(scenario)
                } finally {
                    scenario.onActivity { it.editor.cancelTool(); it.editor.workflows.activate(EditorWorkflowPanel.Layers, false) }
                    await(scenario, "strokes-saved-to-close", timeoutMs = 180_000) { it.editor.pending == 0 && !it.editor.busy }
                    scenario.onActivity { if (it.editor.screen != WorkbenchScreen.Projects) it.editor.navigate(WorkbenchScreen.Projects) }
                    await(scenario, "page-closed", timeoutMs = 120_000) { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
                    storageClosed = true
                }
                closed(id)
            }
        } finally {
            owned?.let { id ->
                val target = File(projectPath(id))
                if (storageClosed) assertTrue(!target.exists() || target.deleteRecursively())
            }
        }
    }

    /** What a burst left behind: how many of its strokes the closed project
     * holds; how the document the editor held differs from a read of the
     * project (null: not at all); and what the layer controls showed instead
     * of the list of that document (null: they loaded it). */
    private class Burst(val saved: Int, val heldDiffers: String?, val controls: String?)

    /** Draws [count] strokes onto a page that already holds [filled]. */
    private fun burst(filled: Int, count: Int, strokeMs: Long, gapMs: Long, drawn: (ActivityScenario<MainActivity>) -> Unit = {}): Burst {
        var held: DocumentSnapshot? = null
        var controls: String? = null
        var read: DocumentSnapshot? = null
        withPage(filled, body = { scenario -> drawStrokes(scenario, count, strokeMs, gapMs); drawn(scenario) },
            settled = { scenario ->
                controls = layerControlsIssue(scenario)
                scenario.onActivity { held = it.editor.document }
            },
            closed = { id -> read = savedDocument(id) })
        val saved = checkNotNull(read)
        return Burst(saved.render.items.count { it.shape is Shape.Stroke } - filled, difference(held, saved), controls)
    }

    /** The count first, so that a failure always says how many strokes were
     * saved; then what the editor held and its controls. */
    private fun assertAllSaved(what: String, count: Int, burst: Burst) {
        assertEquals("Saved ${burst.saved} of $count $what", count, burst.saved)
        assertNull("Saved ${burst.saved} of $count $what, but the document the editor held is not the one a read returns: ${burst.heldDiffers}", burst.heldDiffers)
        assertNull("Saved ${burst.saved} of $count $what, but the layer controls were not bound to the document shown: ${burst.controls}", burst.controls)
    }

    @Test fun fourHundredRapidStrokesOnAFreshPageAreAllSaved() {
        val count = 400
        assertAllSaved("rapid strokes (40 ms long, 50 ms apart) on a fresh page", count,
            burst(filled = 0, count = count, strokeMs = 40, gapMs = 50))
    }

    @Test fun strokesAtAWritingPaceOnAPageOfTwelveHundredStrokesAreAllSaved() {
        val count = 80
        assertAllSaved("strokes (70 ms long, 150 ms apart) on a page that held 1,200", count,
            burst(filled = 1_200, count = count, strokeMs = 70, gapMs = 150))
    }

    @Test fun noPenDownForInkIsRefusedWhileEarlierStrokesAreBeingSaved() {
        val count = 150
        var accepted = -1L; var refused = -1L; var report = ""
        val burst = burst(filled = 0, count = count, strokeMs = 40, gapMs = 50) { scenario ->
            scenario.onActivity { activity ->
                // Read by name, so that this file also compiles against a build
                // that does not count pen-downs; there this test fails.
                val metrics = activity.editor.diagnostics.pen
                val counts = metrics.javaClass.methods.firstOrNull { it.name == "penDowns" && it.parameterCount == 0 }?.invoke(metrics)
                if (counts != null) {
                    accepted = counts.javaClass.getMethod("getAccepted").invoke(counts) as Long
                    refused = counts.javaClass.getMethod("getRefusedTotal").invoke(counts) as Long
                }
                report = metrics.report(emptyList())
            }
        }
        val saved = burst.saved
        assertTrue("This build does not count refused pen-downs. Saved $saved of $count strokes", accepted >= 0)
        assertEquals("Pen-downs for ink were refused; saved $saved of $count strokes. $report", 0L, refused)
        assertEquals("Not every pen-down became a stroke; saved $saved of $count strokes. $report", count.toLong(), accepted)
        assertAllSaved("rapid strokes although none was refused. $report", count, burst)
    }
}
