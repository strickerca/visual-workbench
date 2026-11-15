package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import java.lang.reflect.Proxy
import kotlinx.coroutines.runBlocking
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

    @Test fun ink_before_save_switched_off_begins_the_stroke_with_only_the_original_call(): Unit = runBlocking {
        for (capabilities in listOf(true, false)) {
            val calls = mutableListOf<String>()
            assertSame(original, beginInkStroke(project(calls, capabilities), options, lockFree = false))
            assertEquals(listOf("beginStroke:gesture"), calls)
        }
    }

    @Test fun ink_before_save_switched_on_begins_early_and_falls_back_to_the_original_call(): Unit = runBlocking {
        val calls = mutableListOf<String>()
        assertSame(deferred, beginInkStroke(project(calls, true), options, lockFree = true))
        assertEquals(listOf("beginStrokeDeferred:gesture"), calls)
        // A highlighter has no early start: the core answers null.
        calls.clear()
        assertSame(original, beginInkStroke(project(calls, true, early = null), options, lockFree = true))
        assertEquals(listOf("beginStrokeDeferred:gesture", "beginStroke:gesture"), calls)
        calls.clear()
        assertSame(original, beginInkStroke(project(calls, false), options, lockFree = true))
        assertEquals(listOf("beginStroke:gesture"), calls)
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
