package com.visualworkbench.android.editor

import com.visualworkbench.shared.WorkflowBinding

/** The page a plain background is decoded for: the editor's attachment epoch,
 * the refresh fence's epoch (page navigation moves it, ordered changes do not)
 * and the document page. */
internal data class BackgroundPage(val epoch: Long, val admission: Long, val documentId: String, val page: UInt?)

/** What one plain background decode reads: its page, the revision shown when
 * it started, and whether untagged colors are approved as sRGB. */
internal data class PlainBackgroundRequest(val page: BackgroundPage, val binding: WorkflowBinding, val assumed: Boolean)

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
 * A newer revision of the same page adopted during the decode, or a color
 * approval given meanwhile, drops the result, and the same load decodes once
 * more for what is shown now ([again]); the refresh that adopted that revision
 * has already run, so nothing else would. One decode per page runs at a time
 * ([begin]): a load that finds one in flight leaves the page to it, and that
 * one checks the request again when it ends. A failure for an unchanged
 * request is shown, and refreshes do not repeat it. */
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

    /** Whether the load must decode the same page again instead of adopting
     * or reporting what it has. */
    fun again(started: PlainBackgroundRequest, now: PlainBackgroundRequest?): Boolean =
        now != null && now.page == started.page && (now.binding != started.binding || now.assumed != started.assumed)
}
