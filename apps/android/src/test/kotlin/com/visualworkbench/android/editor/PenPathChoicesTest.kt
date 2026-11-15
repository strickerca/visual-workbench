package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import java.lang.reflect.Proxy
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.yield
import org.junit.Assert.*
import org.junit.Test

/** Each canvas developer switch chooses between the original call and a new
 * one in a single place. Switched off, that place makes exactly the original
 * call and nothing else; these tests record the calls to show it. */
class PenPathChoicesTest {
    private val revision = ProjectInfo("project", "title", "device", 1uL, false, false, 0uL, "hash", listOf("doc"))
    private fun snapshot(title: String) = DocumentSnapshot("doc", title, 16u, 16u, 8u, emptyList(), RenderList(revision, emptyList()))
    private val held = snapshot("held")
    private val full = snapshot("full read")
    private val changed = snapshot("changes read")
    private val options = StrokeOptions("gesture", "transaction", "object", "doc", "layer", "device", 1uL, 0L, "pen", 4.0, 0x203040ffu)
    private fun stroke(): WorkbenchStroke = Proxy.newProxyInstance(WorkbenchStroke::class.java.classLoader,
        arrayOf(WorkbenchStroke::class.java)) { _, method, _ -> error("unexpected stroke call ${method.name}") } as WorkbenchStroke
    private val original = stroke()
    private val deferred = stroke()

    /** A project that records the calls made to it. With [capabilities] it also
     * offers the two optional calls the switches use; [changes] and [early] are
     * their answers, null meaning "not for this document" or "not for this tool". */
    private fun project(calls: MutableList<String>, capabilities: Boolean, changes: DocumentSnapshot? = changed,
        early: WorkbenchStroke? = deferred): WorkbenchProject {
        val offered = if (capabilities) arrayOf(WorkbenchProject::class.java, WorkbenchDocumentChanges::class.java, WorkbenchDeferredStrokes::class.java)
            else arrayOf(WorkbenchProject::class.java)
        return Proxy.newProxyInstance(WorkbenchProject::class.java.classLoader, offered) { _, method, arguments ->
            when (method.name) {
                "document" -> { calls += "document:${arguments[0]}"; full }
                "documentChanges" -> { calls += "documentChanges:${arguments[0]}:${(arguments[1] as DocumentSnapshot).title}"; changes }
                "beginStroke" -> { calls += "beginStroke:${(arguments[0] as StrokeOptions).gestureId}"; original }
                "beginStrokeDeferred" -> { calls += "beginStrokeDeferred:${(arguments[0] as StrokeOptions).gestureId}"; early }
                else -> error("unexpected project call ${method.name}")
            }
        } as WorkbenchProject
    }

    @Test fun quick_refresh_switched_off_makes_only_the_original_full_read(): Unit = runBlocking {
        for (capabilities in listOf(true, false)) {
            val calls = mutableListOf<String>()
            assertSame(full, readDocumentSnapshot(project(calls, capabilities), held, quick = false))
            assertEquals(listOf("document:doc"), calls)
        }
    }

    @Test fun quick_refresh_switched_on_asks_for_changes_and_falls_back_to_the_full_read(): Unit = runBlocking {
        val calls = mutableListOf<String>()
        assertSame(changed, readDocumentSnapshot(project(calls, true), held, quick = true))
        assertEquals(listOf("documentChanges:doc:held"), calls)
        // A paged document has no changes read: the core answers null.
        calls.clear()
        assertSame(full, readDocumentSnapshot(project(calls, true, changes = null), held, quick = true))
        assertEquals(listOf("documentChanges:doc:held", "document:doc"), calls)
        // A project without the optional call is read the original way.
        calls.clear()
        assertSame(full, readDocumentSnapshot(project(calls, false), held, quick = true))
        assertEquals(listOf("document:doc"), calls)
    }

    @Test fun ink_before_save_switched_off_asks_the_project_for_nothing_before_the_lock() {
        // Nothing is begun early; the stroke begins with the original call, under the lock.
        for (capabilities in listOf(true, false)) {
            val calls = mutableListOf<String>()
            assertNull(earlyInkStroke(false, project(calls, capabilities), options))
            assertTrue(calls.isEmpty())
        }
    }

    @Test fun ink_before_save_switched_on_begins_early_or_says_to_take_the_original_path() {
        val calls = mutableListOf<String>()
        assertSame(deferred, earlyInkStroke(true, project(calls, true), options))
        assertEquals(listOf("beginStrokeDeferred:gesture"), calls)
        // A highlighter may have to create its layer from what the project
        // holds, and two of them begun side by side would each create one. It
        // has no early start: null, and nothing else is asked here. The caller
        // then takes the original path, whole, under the lock.
        calls.clear()
        assertNull(earlyInkStroke(true, project(calls, true, early = null), options))
        assertEquals(listOf("beginStrokeDeferred:gesture"), calls)
        calls.clear()
        assertNull(earlyInkStroke(true, project(calls, false), options))
        assertTrue(calls.isEmpty())
    }

    private suspend fun settle() { repeat(50) { yield() } }

    @Test fun a_stroke_inks_at_once_and_is_saved_in_the_order_the_pens_went_down(): Unit = runBlocking {
        val lock = Mutex(); val log = mutableListOf<String>()
        // S0 is finished and its save is slow (a recovery checkpoint).
        val slow = CompletableDeferred<Unit>()
        val s0 = launch(start = CoroutineStart.UNDISPATCHED) {
            assertEquals("s0", inkThenSaveInOrder(lock, ink = { "s0" }, save = { log += "save:$it:started"; slow.await(); log += "save:$it"; it }))
        }
        settle()
        // S1's pen goes down while S0 is still saving: its ink does not wait.
        val s1Up = CompletableDeferred<String>()
        val s1 = launch(start = CoroutineStart.UNDISPATCHED) {
            assertEquals("s1", inkThenSaveInOrder(lock, ink = { log += "ink:s1"; s1Up.await() }, save = { log += "save:$it"; it }))
        }
        assertEquals(listOf("save:s0:started", "ink:s1"), log)
        s1Up.complete("s1"); settle() // S1 is finished and waits for S0's save.
        // An edit is asked for now, and then the editor is closed.
        val edit = launch(start = CoroutineStart.UNDISPATCHED) { lock.withLock { log += "edit" } }
        val close = launch(start = CoroutineStart.UNDISPATCHED) { lock.withLock { log += "close" } }
        settle(); assertEquals(listOf("save:s0:started", "ink:s1"), log)
        slow.complete(Unit); joinAll(s0, s1, edit, close)
        // S1 was drawn before the edit was asked for and is saved before it; closing comes last.
        assertEquals(listOf("save:s0:started", "ink:s1", "save:s0", "save:s1", "edit", "close"), log)
        assertFalse(lock.isLocked)
    }

    @Test fun a_stroke_still_being_drawn_holds_its_place_against_an_edit_asked_for_meanwhile(): Unit = runBlocking {
        val lock = Mutex(); val log = mutableListOf<String>()
        val up = CompletableDeferred<String>()
        val stroke = launch(start = CoroutineStart.UNDISPATCHED) { inkThenSaveInOrder(lock, ink = { up.await() }, save = { log += "save:$it"; it }) }
        val edit = launch(start = CoroutineStart.UNDISPATCHED) { lock.withLock { log += "edit" } }
        settle(); assertTrue(log.isEmpty()) // As on the original path, where the stroke holds the lock while it is drawn.
        up.complete("stroke"); joinAll(stroke, edit)
        assertEquals(listOf("save:stroke", "edit"), log)
    }

    @Test fun a_cancelled_stroke_gives_its_place_up_and_waits_for_nobody(): Unit = runBlocking {
        val lock = Mutex(); lock.lock() // An earlier save is under way.
        var saved = 0
        assertNull(inkThenSaveInOrder<String, String>(lock, ink = { null }, save = { saved++; it }))
        assertTrue(lock.isLocked); assertEquals(0, saved)
        lock.unlock()
        // The place already holds the lock while the stroke is drawn; cancelling the stroke frees it.
        val gate = CompletableDeferred<String?>()
        val drawing = launch(start = CoroutineStart.UNDISPATCHED) { assertNull(inkThenSaveInOrder<String, String>(lock, ink = { gate.await() }, save = { saved++; it })) }
        settle(); assertTrue(lock.isLocked)
        gate.complete(null); drawing.join()
        assertFalse(lock.isLocked); assertEquals(0, saved)
        // A save that finds nothing to save any more (cancelled at the last moment) answers null.
        assertNull(inkThenSaveInOrder<String, String>(lock, ink = { "late" }, save = { null }))
        assertFalse(lock.isLocked)
    }

    @Test fun a_failure_of_the_ink_or_of_the_save_is_thrown_and_frees_the_lock(): Unit = runBlocking {
        val lock = Mutex()
        try { inkThenSaveInOrder<String, String>(lock, ink = { throw IllegalStateException("ink") }, save = { it }); fail("the ink failure was swallowed") }
        catch (error: IllegalStateException) { assertEquals("ink", error.message) }
        assertFalse(lock.isLocked)
        try { inkThenSaveInOrder<String, String>(lock, ink = { "stroke" }, save = { throw IllegalArgumentException("save") }); fail("the save failure was swallowed") }
        catch (error: IllegalArgumentException) { assertEquals("save", error.message) }
        assertFalse(lock.isLocked)
        assertEquals("next", inkThenSaveInOrder(lock, ink = { "next" }, save = { it }))
    }

    @Test fun ink_before_save_switched_off_never_lets_a_pen_down_past_the_list_refresh() {
        // The original gate: no tool and no state passes, and the eraser route is not even asked.
        for (tool in EditorTool.entries) for (assisted in listOf(false, true)) for (text in listOf(false, true)) for (region in listOf(false, true))
            assertFalse(inkPassesListRefresh(false, tool, assisted, text, region) { fail("the eraser route must not be asked"); EraserInputRoute.Editor })
    }

    @Test fun ink_before_save_switched_on_lets_only_a_plain_ink_stroke_past_the_list_refresh() {
        val ink = setOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker)
        for (tool in EditorTool.entries) {
            assertEquals(tool.name, tool in ink, inkPassesListRefresh(true, tool, false, false, false) { EraserInputRoute.Editor })
            // An assisted selection prompt, the text tool and the region tool use the pen for something else.
            assertFalse(inkPassesListRefresh(true, tool, true, false, false) { EraserInputRoute.Editor })
            assertFalse(inkPassesListRefresh(true, tool, false, true, false) { EraserInputRoute.Editor })
            assertFalse(inkPassesListRefresh(true, tool, false, false, true) { EraserInputRoute.Editor })
            // The pen's eraser end, an eraser selection and a blocked eraser are not ink.
            for (route in EraserInputRoute.entries) if (route != EraserInputRoute.Editor)
                assertFalse(inkPassesListRefresh(true, tool, false, false, false) { route })
        }
    }

    @Test fun draw_only_the_new_stroke_switched_off_draws_the_whole_scene_without_asking() {
        assertTrue(wholeSceneNeeded(keepDryPicture = false) { fail("the ink-only drawing must not be tried"); true })
    }

    @Test fun draw_only_the_new_stroke_switched_on_draws_the_whole_scene_only_when_ink_only_cannot() {
        var asked = 0
        assertFalse(wholeSceneNeeded(keepDryPicture = true) { asked++; true })
        assertTrue(wholeSceneNeeded(keepDryPicture = true) { asked++; false })
        assertEquals(2, asked)
    }
}
