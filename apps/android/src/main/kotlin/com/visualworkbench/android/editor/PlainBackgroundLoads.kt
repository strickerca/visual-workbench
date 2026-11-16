package com.visualworkbench.android.editor

import com.visualworkbench.android.session.RefreshAdmission
import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.WorkflowBinding
import com.visualworkbench.shared.giantDisplayRequired
import com.visualworkbench.shared.originalFormatRevisionMatches

/** The page a plain background is decoded for: the editor's attachment epoch,
 * the refresh fence's epoch (page navigation moves it, ordered changes do not)
 * and the document page. */
internal data class BackgroundPage(val epoch: Long, val admission: Long, val documentId: String, val page: UInt?)

/** What one plain background decode reads: its page, the revision shown when
 * it started, and the sRGB consent the user recorded (the approved source
 * asset, or null). The consent is read as recorded, not through the source
 * information a refresh clears and reads again. */
internal data class PlainBackgroundRequest(val page: BackgroundPage, val binding: WorkflowBinding, val consent: String?)

/** How a plain pass that neither adopted nor reported ends. */
internal enum class PlainPass {
    Done,
    /** The user's consent changed during the decode: decode again now. */
    Again,
    /** A refresh adopted a newer revision of the page: decode again once that
     * refresh has finished, so its source information is read. */
    AfterRefresh,
}

/** Main-confined decisions for a plain page background: no saved tone, no
 * tiled display, no document page rendering, no stream draft.
 *
 * Its pixels are the page's original. A finished decode belongs to the page
 * for as long as the revision it read is shown, even when an ordered change
 * was observed meanwhile ([adopts]). Such a change queues a refresh, and for a
 * plain page that refresh keeps the background without decoding again, so
 * dropping the decode left the page without one. The core replays its latest
 * change to each new subscriber on its own thread, so on a loaded device the
 * Opened replay could land during the first decode after open.
 *
 * A newer revision of the same page adopted during the decode, or a consent
 * recorded meanwhile, drops the result, and the same load decodes once more
 * ([next], [load]); the refresh that adopted that revision has already run,
 * so nothing else would. One decode per page runs at a time ([begin]): a load
 * that finds one in flight leaves the page to it, and that one checks the
 * request again when it ends. A failure for an unchanged request is shown,
 * and refreshes do not repeat it. */
internal class PlainBackgroundLoads {
    private var running: BackgroundPage? = null

    /** Admits one decode for [page]; false while one for it is in flight. */
    fun begin(page: BackgroundPage): Boolean {
        if (running == page) return false
        running = page
        return true
    }

    fun end(page: BackgroundPage) { if (running == page) running = null }

    /** [now] is the current page's request, or null once the editor shows
     * anything else. */
    fun adopts(started: PlainBackgroundRequest, now: PlainBackgroundRequest?): Boolean =
        now != null && now.page == started.page && now.binding == started.binding

    /** What a pass does instead of adopting or reporting what it has. */
    fun next(started: PlainBackgroundRequest, now: PlainBackgroundRequest?): PlainPass = when {
        now == null || now.page != started.page -> PlainPass.Done
        now.binding != started.binding -> PlainPass.AfterRefresh
        now.consent != started.consent -> PlainPass.Again
        else -> PlainPass.Done
    }

    /** Runs one load's passes. Each further pass follows a decode that a newer
     * revision or consent overtook; nothing polls. [afterRefresh] waits for
     * the refresh that adopted the newer revision: until it ends, the source
     * information the color approval is checked against still belongs to the
     * previous revision or is being read. */
    suspend fun load(pass: suspend () -> PlainPass, afterRefresh: suspend () -> Unit) {
        while (true) when (pass()) {
            PlainPass.Done -> return
            PlainPass.Again -> Unit
            PlainPass.AfterRefresh -> afterRefresh()
        }
    }

    companion object {
        /** The request for [doc] when the editor shows it as a plain page;
         * null for no document, a detached or closing editor, a document
         * page, a stream draft, saved tone or a tiled original. */
        fun request(doc: DocumentSnapshot?, attached: Boolean, documentPage: Boolean, streamDraft: Boolean,
            epoch: Long, admission: Long, consent: String?): PlainBackgroundRequest? {
            if (doc == null || !attached || documentPage || streamDraft || doc.hasToneAdjustments || giantDisplayRequired(doc)) return null
            val revision = doc.render.revision
            return PlainBackgroundRequest(BackgroundPage(epoch, admission, doc.documentId, doc.page),
                WorkflowBinding(revision.projectId, doc.documentId, revision.hostSeq, revision.stateHash), consent)
        }
    }
}

/** Whether a source-information receipt read for [binding] under [ticket]
 * still belongs to what [shown] displays: the same project and page (the
 * fence's epoch) and that document at that revision. An ordered change alone
 * (the core replays Opened to each new subscriber) does not stale it; the
 * refresh it queues reads the receipt again. */
internal fun keepsFormatReceipt(fence: RefreshAdmission, ticket: RefreshAdmission.Ticket,
    shown: DocumentSnapshot?, binding: WorkflowBinding): Boolean =
    fence.judge(ticket) != RefreshAdmission.Verdict.Dropped && shown != null &&
        shown.documentId == binding.documentId && originalFormatRevisionMatches(shown.render.revision, binding)
