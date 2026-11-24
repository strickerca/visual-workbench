package com.visualworkbench.android.editor

import com.visualworkbench.android.diagnostics.PenRefusal
import com.visualworkbench.shared.*
import java.lang.reflect.Proxy
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.yield
import org.junit.Assert.*
import org.junit.Test

/** The one path a pen stroke takes: the pen is never refused for what the
 * editor is still doing for earlier strokes, ink starts at once, and strokes
 * are saved in the order the pens went down. */
class InkStrokesTest {
    private val revision = ProjectInfo("project", "title", "device", 1uL, false, false, 0uL, "hash", listOf("doc"))
    private fun snapshot(title: String) = DocumentSnapshot("doc", title, 16u, 16u, 8u, emptyList(), RenderList(revision, emptyList()))
    private val held = snapshot("held")
    private val full = snapshot("full read")
    private val changed = snapshot("changes read")

    /** A project that records the reads made of it. With [changes] it offers
     * the read that rebuilds only what changed; [answer] is what that read
     * returns, null meaning "not for this document". */
    private fun project(calls: MutableList<String>, changes: Boolean, answer: DocumentSnapshot? = changed): WorkbenchProject {
        val offered = if (changes) arrayOf(WorkbenchProject::class.java, WorkbenchDocumentChanges::class.java) else arrayOf(WorkbenchProject::class.java)
        return Proxy.newProxyInstance(WorkbenchProject::class.java.classLoader, offered) { _, method, arguments ->
            when (method.name) {
                "document" -> { calls += "document:${arguments[0]}"; full }
                "documentChanges" -> { calls += "documentChanges:${arguments[0]}:${(arguments[1] as DocumentSnapshot?)?.title}"; answer }
                else -> error("unexpected project call ${method.name}")
            }
        } as WorkbenchProject
    }

    /** Nothing stands in the way of the pen. */
    private val open = PenDownState(streamOwnsInput = false, busy = false, modal = false, listsBusy = false, pictureMissing = false,
        strokesInFlight = 0, unsavedSamples = 0)

    @Test fun ink_is_taken_while_earlier_strokes_are_still_saved_and_the_lists_are_read_again() {
        assertNull(penDownRefusal(open, ink = true))
        // After every save the instruction and marker lists are read again.
        assertNull(penDownRefusal(open.copy(listsBusy = true), ink = true))
        // Earlier strokes are drawn, waiting for their save, or just saved:
        // far more of them than the three the editor used to stop at.
        for (inFlight in listOf(1, 2, 3, 4, 10, MAX_STROKES_IN_FLIGHT - 1))
            assertNull("strokes in flight: $inFlight", penDownRefusal(open.copy(listsBusy = true, strokesInFlight = inFlight, unsavedSamples = inFlight * 400L), ink = true))
        // One full-length stroke still unsaved does not stop the next one either.
        assertNull(penDownRefusal(open.copy(strokesInFlight = 1, unsavedSamples = MAX_STROKE_SAMPLES.toLong()), ink = true))
    }

    @Test fun anything_that_edits_what_is_shown_still_waits_for_the_lists() {
        assertNull(penDownRefusal(open, ink = false))
        assertEquals(PenRefusal.Lists, penDownRefusal(open.copy(listsBusy = true), ink = false))
    }

    @Test fun the_only_refusal_for_unsaved_ink_is_the_bound_on_what_is_held() {
        for (ink in listOf(true, false)) {
            assertEquals(PenRefusal.Backlog, penDownRefusal(open.copy(strokesInFlight = MAX_STROKES_IN_FLIGHT), ink))
            assertEquals(PenRefusal.Backlog, penDownRefusal(open.copy(strokesInFlight = 4, unsavedSamples = MAX_UNSAVED_SAMPLES), ink))
            assertNull(penDownRefusal(open.copy(strokesInFlight = 4, unsavedSamples = MAX_UNSAVED_SAMPLES - 1), ink))
        }
        // The bounds are what the core and memory allow, not a pace of drawing:
        // every one of the core's ink workers but the brush preview's, and the
        // samples of four full-length strokes.
        assertEquals(63, MAX_STROKES_IN_FLIGHT)
        assertEquals(400_000L, MAX_UNSAVED_SAMPLES)
        assertEquals(100_000, MAX_STROKE_SAMPLES)
    }

    @Test fun a_state_in_which_nothing_may_be_drawn_refuses_ink_too_and_names_itself() {
        for (ink in listOf(true, false)) {
            assertEquals(PenRefusal.Stream, penDownRefusal(open.copy(streamOwnsInput = true), ink))
            assertEquals(PenRefusal.Busy, penDownRefusal(open.copy(busy = true), ink))
            assertEquals(PenRefusal.Modal, penDownRefusal(open.copy(modal = true), ink))
            assertEquals(PenRefusal.Loading, penDownRefusal(open.copy(pictureMissing = true), ink))
        }
        // The first reason that holds is the one reported.
        assertEquals(PenRefusal.Stream, penDownRefusal(PenDownState(true, true, true, true, true, MAX_STROKES_IN_FLIGHT, MAX_UNSAVED_SAMPLES), ink = true))
        assertEquals(PenRefusal.Loading, penDownRefusal(open.copy(pictureMissing = true, strokesInFlight = MAX_STROKES_IN_FLIGHT), ink = true))
    }

    @Test fun only_a_plain_ink_stroke_asks_for_ink() {
        val ink = setOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker)
        assertEquals(ink, INK_TOOLS)
        for (tool in EditorTool.entries) {
            assertEquals(tool.name, tool in ink, asksForInk(tool, false, false, false) { EraserInputRoute.Editor })
            // An assisted selection prompt, the text tool and the region tool use the pen for something else.
            assertFalse(asksForInk(tool, true, false, false) { EraserInputRoute.Editor })
            assertFalse(asksForInk(tool, false, true, false) { EraserInputRoute.Editor })
            assertFalse(asksForInk(tool, false, false, true) { EraserInputRoute.Editor })
            // The pen's eraser end, an eraser selection and a blocked eraser are not ink.
            for (route in EraserInputRoute.entries) if (route != EraserInputRoute.Editor)
                assertFalse(asksForInk(tool, false, false, false) { route })
        }
    }

    private suspend fun settle() { repeat(50) { yield() } }

    @Test fun a_stroke_inks_at_once_and_is_saved_in_the_order_the_pens_went_down(): Unit = runBlocking {
        val lock = Mutex(); val log = mutableListOf<String>()
        // S0 is finished and its save is slow (a busy disk).
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

    @Test fun many_strokes_drawn_while_one_save_is_slow_all_ink_at_once_and_are_saved_in_order(): Unit = runBlocking {
        val lock = Mutex(); val inked = mutableListOf<Int>(); val saved = mutableListOf<String>()
        val slow = CompletableDeferred<Unit>()
        val jobs = mutableListOf<Job>()
        jobs += launch(start = CoroutineStart.UNDISPATCHED) {
            inkThenSaveInOrder(lock, ink = { 0 }, save = { slow.await(); saved += "stroke:$it"; it })
        }
        settle()
        // Sixty strokes are drawn, one after the other, while that save has not
        // finished. Each has its ink the moment its pen goes down; an edit and
        // an undo are asked for among them.
        val count = MAX_STROKES_IN_FLIGHT - 3
        for (number in 1..count) {
            val up = CompletableDeferred<Int>()
            jobs += launch(start = CoroutineStart.UNDISPATCHED) {
                inkThenSaveInOrder(lock, ink = { inked += number; up.await() }, save = { saved += "stroke:$it"; it })
            }
            assertEquals("stroke $number has ink at pen-down", number, inked.last())
            up.complete(number)
            if (number == 20) jobs += launch(start = CoroutineStart.UNDISPATCHED) { lock.withLock { saved += "edit" } }
            if (number == 40) jobs += launch(start = CoroutineStart.UNDISPATCHED) { lock.withLock { saved += "undo" } }
        }
        settle()
        assertEquals((1..count).toList(), inked)
        assertTrue("nothing is saved ahead of the slow save", saved.isEmpty())
        slow.complete(Unit); jobs.joinAll()
        val expected = buildList {
            add("stroke:0")
            for (number in 1..count) { add("stroke:$number"); if (number == 20) add("edit"); if (number == 40) add("undo") }
        }
        assertEquals(expected, saved)
        assertFalse(lock.isLocked)
    }

    @Test fun a_stroke_that_begins_in_its_turn_keeps_its_place_and_the_ones_after_it_still_ink(): Unit = runBlocking {
        // A highlighter cannot begin before the project has answered: it hands
        // over at once and begins, inks and saves under the lock, in its turn.
        val lock = Mutex(); val log = mutableListOf<String>()
        val slow = CompletableDeferred<Unit>()
        val first = launch(start = CoroutineStart.UNDISPATCHED) { inkThenSaveInOrder(lock, ink = { "pen1" }, save = { slow.await(); log += "save:$it"; it }) }
        settle()
        val highlighter = launch(start = CoroutineStart.UNDISPATCHED) {
            inkThenSaveInOrder(lock, ink = { "highlighter" }, save = { log += "begin:$it"; log += "save:$it"; it })
        }
        val penUp = CompletableDeferred<String>()
        val later = launch(start = CoroutineStart.UNDISPATCHED) { inkThenSaveInOrder(lock, ink = { log += "ink:pen2"; penUp.await() }, save = { log += "save:$it"; it }) }
        assertEquals(listOf("ink:pen2"), log)
        penUp.complete("pen2"); slow.complete(Unit); joinAll(first, highlighter, later)
        assertEquals(listOf("ink:pen2", "save:pen1", "begin:highlighter", "save:highlighter", "save:pen2"), log)
    }

    @Test fun a_stroke_still_being_drawn_holds_its_place_against_an_edit_asked_for_meanwhile(): Unit = runBlocking {
        val lock = Mutex(); val log = mutableListOf<String>()
        val up = CompletableDeferred<String>()
        val stroke = launch(start = CoroutineStart.UNDISPATCHED) { inkThenSaveInOrder(lock, ink = { up.await() }, save = { log += "save:$it"; it }) }
        val edit = launch(start = CoroutineStart.UNDISPATCHED) { lock.withLock { log += "edit" } }
        settle(); assertTrue(log.isEmpty())
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

    @Test fun a_refresh_asks_for_changes_and_only_reads_everything_where_the_project_cannot(): Unit = runBlocking {
        var calls = mutableListOf<String>()
        assertSame(changed, readDocument(project(calls, changes = true), "doc", held))
        assertEquals(listOf("documentChanges:doc:held"), calls)
        // A first read holds nothing yet.
        calls = mutableListOf()
        assertSame(changed, readDocument(project(calls, changes = true), "doc", null))
        assertEquals(listOf("documentChanges:doc:null"), calls)
        // A paged document, or one at the size limit of a read, has no such path.
        calls = mutableListOf()
        assertSame(full, readDocument(project(calls, changes = true, answer = null), "doc", held))
        assertEquals(listOf("documentChanges:doc:held", "document:doc"), calls)
        // Nor has a project without the call.
        calls = mutableListOf()
        assertSame(full, readDocument(project(calls, changes = false), "doc", held))
        assertEquals(listOf("document:doc"), calls)
    }
}
