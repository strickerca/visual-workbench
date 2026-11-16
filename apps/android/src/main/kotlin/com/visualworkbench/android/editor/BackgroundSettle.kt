package com.visualworkbench.android.editor

import com.visualworkbench.shared.WorkbenchProject
import com.visualworkbench.shared.WorkbenchSettling
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** What the editor does for its open project when the app leaves the screen.
 *
 * It does not close the project: the user comes back to the same editor.
 * Android freezes a cached app's process about ten seconds after it leaves
 * the screen and ends it at will, and the core's own upkeep is frozen with
 * it. A recovery checkpoint that is due, or one the project would write after
 * ten quiet seconds, would then never be written, and the next open would
 * replay more of the log (nothing is lost: the log holds every edit). So the
 * core is asked to write now the checkpoint closing would write.
 *
 * The request runs in [scope], the editor's own, which outlives the paused
 * activity, and the core call suspends off the main thread. It waits until
 * the editor is [idle]: no stroke drawn or saved, no edit, open or close in
 * flight. The core itself refuses it while a stroke is being drawn and runs
 * it after the project's queued work. */
internal class BackgroundSettle(private val scope: CoroutineScope, private val idle: suspend () -> Boolean) {
    private var settling: Job? = null

    /** On the main thread as the app goes to the background, with the project
     * open now. [current] is the open project once the editor is idle: a
     * project closed or replaced by then is not asked. One request at a time;
     * null when nothing is asked. */
    fun background(project: WorkbenchProject?, current: () -> WorkbenchProject?): Job? {
        val settles = project as? WorkbenchSettling ?: return null
        if (settling?.isActive == true) return null
        return scope.launch {
            if (!idle() || current() !== project) return@launch
            try { settles.settle() }
            catch (cancel: CancellationException) { throw cancel }
            // Drawing again, closed meanwhile, or not written: the log holds
            // every edit, and closing or the next open writes it instead.
            catch (_: Exception) {}
        }.also { settling = it }
    }

    /** On the main thread as the app comes back. A request still waiting for
     * the editor to be idle is dropped, so that it does not pause the user's
     * work; the project writes the checkpoint by itself once it is left
     * alone. One the core has begun to write finishes. */
    fun foreground() {
        settling?.cancel()
        settling = null
    }
}
