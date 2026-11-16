package com.visualworkbench.android.editor

import com.visualworkbench.shared.WorkbenchProject
import com.visualworkbench.shared.WorkbenchSettling
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** What the editor does for its open project once the app is out of sight.
 *
 * It does not close the project: the user comes back to the same editor.
 * Android freezes a cached app's process about ten seconds after it leaves
 * the screen and ends it at will, and the core's own upkeep is frozen with
 * it. A recovery checkpoint that is due, or one the project would write after
 * ten quiet seconds, would then never be written, and the next open would
 * replay more of the log (nothing is lost: the log holds every edit). So the
 * core is asked to write now the checkpoint closing would write.
 *
 * A checkpoint cannot be stopped once begun and holds the project for as
 * long as its history is large (about four seconds at 8,000 strokes). So it
 * is asked for only when the activity is no longer visible at all (onStop:
 * not for a share sheet, a permission prompt or a picker over the editor,
 * which only pause it), and only after [GRACE_MS] more out of sight, so that
 * a quick look at another app or at Recents does not leave a checkpoint for
 * the user to wait out on return. Coming back ([shown]) drops a request that
 * has not reached the core. Two seconds of grace and four of checkpoint
 * still end before the freeze.
 *
 * The request runs in [scope], the editor's own, which outlives the stopped
 * activity, and the core call suspends off the main thread. It waits until
 * the editor is [idle]: no native call of its own in flight. The core itself
 * refuses it while a stroke is being drawn and runs it after the project's
 * queued work. [wait] is the delay, replaced by virtual time in tests. */
internal class BackgroundSettle(private val scope: CoroutineScope, private val idle: suspend () -> Boolean,
    private val wait: suspend (Long) -> Unit = { delay(it) }) {
    private var settling: Job? = null

    /** On the main thread when the activity stops, with the project open now.
     * [current] is the open project once the grace period is over and the
     * editor is idle: a project closed or replaced by then is not asked. One
     * request at a time; null when nothing is asked. */
    fun hidden(project: WorkbenchProject?, current: () -> WorkbenchProject?): Job? {
        val settles = project as? WorkbenchSettling ?: return null
        if (settling?.isActive == true) return null
        return scope.launch {
            wait(GRACE_MS)
            if (!idle() || current() !== project) return@launch
            try { settles.settle() }
            catch (cancel: CancellationException) { throw cancel }
            // Drawing again, closed meanwhile, or not written: the log holds
            // every edit, and closing or the next open writes it instead.
            catch (_: Exception) {}
        }.also { settling = it }
    }

    /** On the main thread when the activity starts again. A request still in
     * its grace period or waiting for the editor to be idle is dropped; the
     * project writes the checkpoint by itself once it is left alone. One the
     * core has begun to write finishes. */
    fun shown() {
        settling?.cancel()
        settling = null
    }

    companion object {
        /** How long the app stays out of sight before the request is made. */
        const val GRACE_MS: Long = 2_000L
    }
}
