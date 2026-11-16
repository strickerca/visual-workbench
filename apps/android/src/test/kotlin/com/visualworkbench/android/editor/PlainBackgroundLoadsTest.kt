package com.visualworkbench.android.editor

import com.visualworkbench.android.session.RefreshAdmission
import com.visualworkbench.shared.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import kotlin.math.max

/** Drives PlainBackgroundLoads (its pass loop, request gating and decisions)
 * and the source-information keep rule with the editor's real refresh fence.
 * The pass itself stands in for loadBackgroundOnce: no decoder, bitmap or
 * event thread runs, and events that land during a decode are applied there. */
class PlainBackgroundLoadsTest {
    private val source = "c".repeat(64)
    private fun info(hostSeq: ULong) = ProjectInfo("project", "image", "device", 1uL, false, false, hostSeq,
        "%064x".format(hostSeq.toLong()), listOf("document"))
    private fun snapshot(hostSeq: ULong, width: UInt = 16u, height: UInt = 16u, tone: Boolean = false, page: UInt? = null) =
        DocumentSnapshot("document", "image", width, height, 8u, emptyList(), RenderList(info(hostSeq), emptyList()), page, tone)
    private fun DocumentSnapshot.binding() = WorkflowBinding(render.revision.projectId, documentId, render.revision.hostSeq, render.revision.stateHash)

    /** Stands in for the editor around one plain page: the document it shows,
     * the source-information receipt it holds (null while a refresh reads it
     * again), the consent recorded, and what its decodes did. */
    private inner class Editor(var shown: DocumentSnapshot = snapshot(1u), var consent: String? = null) {
        val fence = RefreshAdmission().also { it.reset() }
        val loads = PlainBackgroundLoads()
        var receipt: WorkflowBinding? = shown.binding()
        /** Ends the refresh that adopted [shown]: its source information is read. */
        var finishRefresh: () -> Unit = {}
        var decodes = 0; var adoptions = 0; var reported = 0; var inFlight = 0; var mostInFlight = 0
        val assumptions = mutableListOf<Boolean>()
        var background: WorkflowBinding? = null
        /** approvedAssumption: the consent, checked against the shown revision's receipt. */
        fun approved() = consent == source && receipt == shown.binding()
        fun request() = PlainBackgroundLoads.request(shown, true, false, false, 1, fence.capture().epoch, consent)
        /** A refresh adopting [next]: the document changes, then its receipt is
         * read again; a plain page keeps its background and decodes nothing. */
        fun adopt(next: DocumentSnapshot, readNow: Boolean = true) {
            shown = next; receipt = null
            finishRefresh = { receipt = next.binding(); finishRefresh = {} }
            if (readNow) finishRefresh()
        }
        /** One pass in loadBackgroundOnce's order; [fails] decides whether the
         * decode throws given the assumption it used. */
        suspend fun pass(fails: (Boolean) -> Boolean, during: suspend (Int) -> Unit): PlainPass {
            val started = request() ?: return PlainPass.Done
            if (!loads.begin(started.page)) return PlainPass.Done
            try {
                val assumed = approved()
                decodes++; assumptions += assumed; inFlight++; mostInFlight = max(mostInFlight, inFlight)
                try { during(decodes) } finally { inFlight-- }
                val now = request()
                if (fails(assumed)) {
                    loads.next(started, now).let { if (it != PlainPass.Done) return it }
                    if (loads.adopts(started, now)) reported++
                    return PlainPass.Done
                }
                if (loads.adopts(started, now)) { background = started.binding; adoptions++; return PlainPass.Done }
                return loads.next(started, now)
            } finally { loads.end(started.page) }
        }
        suspend fun load(fails: (Boolean) -> Boolean = { false }, during: suspend (Int) -> Unit = {}) =
            loads.load({ pass(fails, during) }) { finishRefresh() } // The controller waits for refreshMutex here.
    }

    /** The core replays Opened to the new subscriber. On a loaded device that
     * replay was observed during the first decode, the fence then refused the
     * decode, and the refresh it queued did not load a plain page again. */
    @Test fun openedReplayDuringTheFirstDecodeKeepsItsBackground() = runBlocking {
        val editor = Editor()
        val ticket = editor.fence.capture()
        editor.load(during = { assertTrue(editor.fence.observe(1u)) })
        assertFalse(editor.fence.accepts(ticket)) // What dropped it before.
        assertEquals(snapshot(1u).binding(), editor.background)
        editor.adopt(snapshot(1u)) // The replay's refresh adopts the same revision.
        assertEquals(1, editor.decodes); assertEquals(1, editor.adoptions); assertEquals(0, editor.reported)
    }

    @Test fun aChangeObservedBeforeItsRefreshAdoptsLeavesTheDecodeAdoptable() = runBlocking {
        val editor = Editor()
        editor.load(during = { editor.fence.observe(2u) }) // A PC edit arrives; its refresh has not run yet.
        editor.adopt(snapshot(2u))
        assertEquals(snapshot(1u).binding(), editor.background) // A plain page keeps its original pixels.
        assertEquals(1, editor.decodes); assertEquals(1, editor.adoptions)
    }

    /** A stale drop, then a refresh: the background is loaded exactly once. */
    @Test fun aRevisionAdoptedDuringTheDecodeDropsItAndTheSameLoadDecodesOnceMore() = runBlocking {
        val editor = Editor()
        editor.load(during = { pass -> if (pass == 1) { editor.fence.observe(2u); editor.adopt(snapshot(2u)) } })
        assertEquals(2, editor.decodes); assertEquals(1, editor.adoptions)
        assertEquals(snapshot(2u).binding(), editor.background)
        editor.fence.observe(3u); editor.adopt(snapshot(3u)) // Later refreshes decode nothing.
        assertEquals(2, editor.decodes); assertEquals(1, editor.adoptions); assertEquals(1, editor.mostInFlight)
    }

    /** sRGB approved for an untagged original, then a PC edit adopts R2. Its
     * refresh changes the document before its source information is read, so
     * the approval reads false until that refresh ends. The next pass waits
     * for it instead of decoding without the approval, which failed and, when
     * the read outlasted a pass, reported a display error with no consent. */
    @Test fun aPassAfterANewRevisionWaitsForItsSourceInformation() = runBlocking {
        val editor = Editor(consent = source)
        var approvedWhenDropped = true
        editor.load(fails = { assumed -> !assumed }, during = { pass ->
            if (pass == 1) { editor.fence.observe(2u); editor.adopt(snapshot(2u), readNow = false); approvedWhenDropped = editor.approved() }
        })
        assertFalse(approvedWhenDropped) // A pass started here would have decoded without the approval.
        assertEquals(listOf(true, true), editor.assumptions)
        assertEquals(2, editor.decodes); assertEquals(0, editor.reported)
        assertEquals(snapshot(2u).binding(), editor.background)
    }

    /** A same-revision refresh clears and reads the receipt again during a
     * decode that fails: the recorded consent did not change, so the failure
     * is reported once instead of costing another decode. */
    @Test fun aReceiptBeingReadAgainIsNotAConsentChange() = runBlocking {
        val editor = Editor(consent = source)
        editor.load(fails = { true }, during = { editor.adopt(snapshot(1u), readNow = false) })
        assertFalse(editor.approved()) // Still being read when the decode ended.
        assertEquals(1, editor.decodes); assertEquals(1, editor.reported); assertNull(editor.background)
    }

    @Test fun aSecondLoadOfTheSamePageNeverStartsASecondDecode() = runBlocking {
        val editor = Editor()
        // A refresh that adopts a newer revision asks for the page while its decode runs.
        editor.load(during = { pass -> if (pass == 1) { editor.adopt(snapshot(2u)); editor.load() } })
        assertEquals(1, editor.mostInFlight); assertEquals(2, editor.decodes); assertEquals(1, editor.adoptions)
        assertEquals(snapshot(2u).binding(), editor.background)
        // Consent given during a decode that fails without it: the refused
        // request is served by the decode in flight, once more, with it.
        val consent = Editor()
        consent.load(fails = { assumed -> !assumed }, during = { pass -> if (pass == 1) { consent.consent = source; consent.load() } })
        assertEquals(1, consent.mostInFlight); assertEquals(listOf(false, true), consent.assumptions); assertEquals(0, consent.reported)
        assertEquals(snapshot(1u).binding(), consent.background)
    }

    @Test fun anotherPageIsAdmittedAndAnOlderDecodeCannotReleaseIt() {
        val loads = PlainBackgroundLoads()
        val old = BackgroundPage(1, 1, "document", null); val next = BackgroundPage(2, 2, "document", null)
        assertTrue(loads.begin(old)); assertFalse(loads.begin(old))
        assertTrue(loads.begin(next)) // A new project or page loads its own background.
        loads.end(old) // The superseded decode finishes later.
        assertFalse(loads.begin(next)); loads.end(next); assertTrue(loads.begin(next))
    }

    /** A decode for another page does not make the editor forget one still
     * running: back on the first page, a second decode of it is refused. */
    @Test fun aRoundTripToAnotherPageDoesNotAdmitASecondDecode() {
        val loads = PlainBackgroundLoads()
        val first = BackgroundPage(1, 1, "document", null); val other = BackgroundPage(1, 1, "other", null)
        assertTrue(loads.begin(first)); assertTrue(loads.begin(other))
        assertFalse(loads.begin(first))
        loads.end(other); assertFalse(loads.begin(first))
        loads.end(first); assertTrue(loads.begin(first))
    }

    /** A blocking message or consent prompt: shown once, never re-decoded by refreshes. */
    @Test fun aFailureForTheShownRequestIsReportedAndNotRetried() = runBlocking {
        val editor = Editor()
        editor.load(fails = { true }, during = { editor.fence.observe(1u) })
        assertEquals(1, editor.decodes); assertEquals(1, editor.reported); assertNull(editor.background)
        editor.fence.observe(2u); editor.adopt(snapshot(2u))
        editor.fence.observe(3u); editor.adopt(snapshot(3u))
        assertEquals(1, editor.decodes); assertEquals(1, editor.reported)
    }

    @Test fun aDecodeForAPageTheEditorLeftIsDroppedSilently() = runBlocking {
        val editor = Editor()
        editor.load(during = { editor.fence.invalidate() }) // Page navigation moves the fence's epoch.
        assertEquals(1, editor.decodes); assertEquals(0, editor.adoptions); assertNull(editor.background)
        val toned = Editor()
        toned.load(fails = { true }, during = { toned.adopt(snapshot(2u, tone = true)) }) // Now a composed page.
        assertEquals(1, toned.decodes); assertEquals(0, toned.reported)
    }

    @Test fun onlyAnAttachedPlainImagePageHasARequest() {
        fun request(doc: DocumentSnapshot?, attached: Boolean = true, documentPage: Boolean = false, stream: Boolean = false) =
            PlainBackgroundLoads.request(doc, attached, documentPage, stream, 4, 5, source)
        val plain = request(snapshot(7u))
        assertEquals(PlainBackgroundRequest(BackgroundPage(4, 5, "document", null), snapshot(7u).binding(), source), plain)
        assertNull(request(null))
        assertNull(request(snapshot(7u), attached = false)) // No project, or closing.
        assertNull(request(snapshot(7u), documentPage = true))
        assertNull(request(snapshot(7u), stream = true))
        assertNull(request(snapshot(7u, tone = true)))
        assertNull(request(snapshot(7u, width = 10_000u, height = 5_001u))) // Over 50 MP: tiled.
        assertNotNull(request(snapshot(7u, width = 10_000u, height = 5_000u)))
    }

    @Test fun aSourceInformationReceiptIsKeptPastAnOrderedChangeOnly() {
        val fence = RefreshAdmission(); fence.reset()
        val shown = snapshot(1u)
        val ticket = fence.capture()
        assertTrue(fence.observe(1u)) // The Opened replay.
        assertTrue(keepsFormatReceipt(fence, ticket, shown, shown.binding()))
        assertFalse(keepsFormatReceipt(fence, ticket, snapshot(2u), shown.binding())) // A newer revision is shown.
        assertFalse(keepsFormatReceipt(fence, ticket, null, shown.binding()))
        assertFalse(keepsFormatReceipt(fence, ticket, shown, shown.binding().copy(documentId = "other")))
        fence.invalidate() // Page navigation.
        assertFalse(keepsFormatReceipt(fence, ticket, shown, shown.binding()))
        val reopened = fence.reset() // Another project.
        assertTrue(keepsFormatReceipt(fence, reopened, shown, shown.binding()))
        assertFalse(keepsFormatReceipt(fence, ticket, shown, shown.binding()))
    }
}
