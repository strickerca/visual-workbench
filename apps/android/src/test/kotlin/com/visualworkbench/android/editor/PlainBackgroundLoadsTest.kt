package com.visualworkbench.android.editor

import com.visualworkbench.android.session.RefreshAdmission
import com.visualworkbench.shared.WorkflowBinding
import org.junit.Assert.*
import org.junit.Test
import kotlin.math.max

/** Drives PlainBackgroundLoads with the editor's real refresh fence, in the
 * order loadBackground calls it. No decoder, bitmap or event thread runs; the
 * events that land during a decode are applied at that point. */
class PlainBackgroundLoadsTest {
    private fun revision(hostSeq: ULong) = WorkflowBinding("project", "document", hostSeq, "%064x".format(hostSeq.toLong()))

    /** Stands in for the editor around one plain page: the request it shows,
     * how often it decoded and adopted, and the most decodes ever in flight. */
    private inner class Editor {
        val fence = RefreshAdmission().also { it.reset() }
        val loads = PlainBackgroundLoads()
        val epoch = 1L
        var shownRevision = revision(1u)
        var assumed = false
        var decodes = 0; var adoptions = 0; var reported = 0
        var inFlight = 0; var mostInFlight = 0
        var background: WorkflowBinding? = null
        fun request() = PlainBackgroundRequest(BackgroundPage(epoch, fence.capture().epoch, "document", null), shownRevision, assumed)
        /** loadBackground for a plain page: [during] runs while decode number
         * `decodes` is in flight; [fails] makes that decode throw. */
        fun load(fails: (PlainBackgroundRequest) -> Boolean = { false }, during: (Int) -> Unit = {}) {
            do {
                val started = request()
                if (!loads.begin(started.page)) return
                val again = try {
                    decodes++; inFlight++; mostInFlight = max(mostInFlight, inFlight)
                    try { during(decodes) } finally { inFlight-- }
                    val now = request()
                    if (fails(started)) {
                        if (loads.again(started, now)) true
                        else { if (loads.adopts(started, now)) reported++; false }
                    } else if (loads.adopts(started, now)) { background = started.binding; adoptions++; false }
                    else loads.again(started, now)
                } finally { loads.end(started.page) }
            } while (again)
        }
        /** The queued refresh adopting [next]. For a plain page it keeps the
         * background and decodes nothing; loadBackground is not called. */
        fun refresh(next: WorkflowBinding) { shownRevision = next }
    }

    /** The core replays Opened to the new subscriber. On a loaded device that
     * replay was observed during the first decode, the fence then refused the
     * decode, and the refresh it queued did not load a plain page again. */
    @Test fun openedReplayDuringTheFirstDecodeKeepsItsBackground() {
        val editor = Editor()
        val ticket = editor.fence.capture()
        editor.load(during = { assertTrue(editor.fence.observe(1u)) })
        assertFalse(editor.fence.accepts(ticket)) // What dropped it before.
        assertEquals(revision(1u), editor.background)
        editor.refresh(revision(1u)) // The replay's refresh adopts the same revision.
        assertEquals(1, editor.decodes); assertEquals(1, editor.adoptions); assertEquals(0, editor.reported)
    }

    @Test fun aChangeObservedBeforeItsRefreshAdoptsLeavesTheDecodeAdoptable() {
        val editor = Editor()
        editor.load(during = { editor.fence.observe(2u) }) // A PC edit arrives; its refresh has not run yet.
        editor.refresh(revision(2u))
        assertEquals(revision(1u), editor.background) // A plain page keeps its original pixels.
        assertEquals(1, editor.decodes); assertEquals(1, editor.adoptions)
    }

    /** A stale drop, then a refresh: the background is loaded exactly once. */
    @Test fun aRevisionAdoptedDuringTheDecodeDropsItAndTheSameLoadDecodesOnceMore() {
        val editor = Editor()
        editor.load(during = { pass -> if (pass == 1) { editor.fence.observe(2u); editor.refresh(revision(2u)) } })
        assertEquals(2, editor.decodes); assertEquals(1, editor.adoptions)
        assertEquals(revision(2u), editor.background)
        editor.fence.observe(3u); editor.refresh(revision(3u)) // Later refreshes decode nothing.
        assertEquals(2, editor.decodes); assertEquals(1, editor.adoptions); assertEquals(1, editor.mostInFlight)
    }

    @Test fun aSecondLoadOfTheSamePageNeverStartsASecondDecode() {
        val editor = Editor()
        // A refresh that adopts a newer revision asks for the page while its decode runs.
        editor.load(during = { pass -> if (pass == 1) { editor.refresh(revision(2u)); editor.load() } })
        assertEquals(1, editor.mostInFlight); assertEquals(2, editor.decodes); assertEquals(1, editor.adoptions)
        assertEquals(revision(2u), editor.background)
        // Color consent given during a decode that fails without it: the
        // refused request is served by the decode in flight, once more.
        val consent = Editor()
        consent.load(fails = { !it.assumed }, during = { pass -> if (pass == 1) { consent.assumed = true; consent.load() } })
        assertEquals(1, consent.mostInFlight); assertEquals(2, consent.decodes); assertEquals(0, consent.reported)
        assertEquals(revision(1u), consent.background)
    }

    @Test fun anotherPageIsAdmittedAndAnOlderDecodeCannotReleaseIt() {
        val loads = PlainBackgroundLoads()
        val old = BackgroundPage(1, 1, "document", null); val next = BackgroundPage(2, 2, "document", null)
        assertTrue(loads.begin(old)); assertFalse(loads.begin(old))
        assertTrue(loads.begin(next)) // A new project or page loads its own background.
        loads.end(old) // The superseded decode finishes later.
        assertFalse(loads.begin(next)); loads.end(next); assertTrue(loads.begin(next))
    }

    /** A blocking message or consent prompt: shown once, never re-decoded by refreshes. */
    @Test fun aFailureForTheShownRequestIsReportedAndNotRetried() {
        val editor = Editor()
        editor.load(fails = { true }, during = { editor.fence.observe(1u) })
        assertEquals(1, editor.decodes); assertEquals(1, editor.reported); assertNull(editor.background)
        editor.fence.observe(2u); editor.refresh(revision(2u))
        editor.fence.observe(3u); editor.refresh(revision(3u))
        assertEquals(1, editor.decodes); assertEquals(1, editor.reported)
    }

    @Test fun aDecodeForAPageTheEditorLeftIsDroppedSilently() {
        val editor = Editor()
        editor.load(during = { editor.fence.invalidate() }) // Page navigation moves the fence's epoch.
        assertEquals(1, editor.decodes); assertEquals(0, editor.adoptions); assertNull(editor.background)
        val failed = Editor()
        failed.load(fails = { true }, during = { failed.fence.invalidate() })
        assertEquals(1, failed.decodes); assertEquals(0, failed.reported)
        assertFalse(PlainBackgroundLoads().again(failed.request(), null)) // Not a plain page any more.
    }
}
