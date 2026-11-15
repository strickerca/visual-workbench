package com.visualworkbench.android.editor

import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.EraserInputRoute
import com.visualworkbench.shared.StrokeOptions
import com.visualworkbench.shared.WorkbenchDeferredStrokes
import com.visualworkbench.shared.WorkbenchDocumentChanges
import com.visualworkbench.shared.WorkbenchProject
import com.visualworkbench.shared.WorkbenchStroke
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

// The places where a canvas developer switch chooses between the original call
// and the new one. They are kept out of the controller so that both choices can
// be tested: with its switch off each one makes exactly the original call and
// nothing else.

/** The document as it is now. With [quick] the core is told which strokes
 * [held] already has and rebuilds only the others; it has no such path for
 * paged documents and then answers null, which falls back to the full read. */
internal suspend fun readDocumentSnapshot(current: WorkbenchProject, held: DocumentSnapshot, quick: Boolean): DocumentSnapshot =
    (if (quick) (current as? WorkbenchDocumentChanges)?.documentChanges(held.documentId, held) else null)
        ?: current.document(held.documentId)

/** With the switch on: the stroke, begun without waiting for the project so
 * that its ink can start at once. Null when the core does not offer that for
 * this stroke (a highlighter, a project too busy to take the request, a
 * project without the call): the caller then takes the original path, whole,
 * under the mutation lock. With the switch off nothing is asked. */
internal fun earlyInkStroke(switchOn: Boolean, current: WorkbenchProject, options: StrokeOptions): WorkbenchStroke? =
    if (switchOn) (current as? WorkbenchDeferredStrokes)?.beginStrokeDeferred(options) else null

/** Runs [ink] at once and [save] under [lock], in the order the calls were
 * made: the place in the lock's queue is taken before [ink] first suspends,
 * exactly where a stroke that inks under the lock takes it. Whatever else
 * waits for the lock afterwards (a later stroke, an edit, undo, closing) runs
 * after this save, and a later stroke's ink never waits for it.
 *
 * [ink] returns what to save, or null when there is nothing to save; the
 * place in the queue is then given up at once, also when the lock was already
 * held for it. A failure of either part releases the lock and is thrown. */
internal suspend fun <N : Any, S> inkThenSaveInOrder(lock: Mutex, ink: suspend () -> N?, save: suspend (N) -> S?): S? = coroutineScope {
    val inked = CompletableDeferred<N>()
    val saving = async(start = CoroutineStart.UNDISPATCHED) { lock.withLock { save(inked.await()) } }
    val finished = ink()
    if (finished == null) { saving.cancel(); null } else { inked.complete(finished); saving.await() }
}

internal val INK_TOOLS = setOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker)

/** Whether a pen-down may pass while the instruction and marker lists are
 * read again after a save. Only a plain ink stroke may, and only with the
 * switch on; [route] is asked last and not at all with the switch off. */
internal inline fun inkPassesListRefresh(switchOn: Boolean, tool: EditorTool, assistedCollecting: Boolean,
    textTool: Boolean, regionTool: Boolean, route: () -> EraserInputRoute): Boolean =
    switchOn && tool in INK_TOOLS && !assistedCollecting && !textTool && !regionTool && route() == EraserInputRoute.Editor

/** One ink frame: true when the whole scene has to be drawn, as it always was.
 * [inkOnly] draws the kept picture with only the new stroke over it and says
 * whether it could; it is asked only when the scene carries the switch. */
internal inline fun wholeSceneNeeded(keepDryPicture: Boolean, inkOnly: () -> Boolean): Boolean =
    !(keepDryPicture && inkOnly())
