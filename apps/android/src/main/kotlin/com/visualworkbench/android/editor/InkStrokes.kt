package com.visualworkbench.android.editor

import com.visualworkbench.android.diagnostics.PenRefusal
import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.EraserInputRoute
import com.visualworkbench.shared.WorkbenchDocumentChanges
import com.visualworkbench.shared.WorkbenchProject
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

// The one path a pen stroke takes: its ink starts at pen-down whatever the
// editor is doing for earlier strokes, it is saved in the order the pens went
// down, and what the editor is still doing for earlier strokes is never a
// reason to refuse the pen. These pieces are kept out of the controller so
// that they can be tested without a phone.

internal val INK_TOOLS = setOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker)

/** Whether a pen-down asks for a plain ink stroke: an ink tool is chosen and
 * nothing else has a claim on the pen (an assisted selection prompt, the text
 * or region tool of a document, the eraser end, a selection or eraser tool).
 * [route] is asked last. */
internal inline fun asksForInk(tool: EditorTool, assistedCollecting: Boolean, textTool: Boolean, regionTool: Boolean,
    route: () -> EraserInputRoute): Boolean =
    tool in INK_TOOLS && !assistedCollecting && !textTool && !regionTool && route() == EraserInputRoute.Editor

/** The samples one stroke may take; the core has the same limit. */
internal const val MAX_STROKE_SAMPLES = 100_000

/** Strokes that may be in flight at once: drawn, waiting to be saved, or saved
 * and not yet released. Each holds one of the core's 64 ink workers until it
 * is released; one is left for the brush preview. */
internal const val MAX_STROKES_IN_FLIGHT = 63

/** Samples that may be held by the strokes in flight together when another
 * pen-down is taken: the memory four full-length strokes hold, which is what
 * the editor could hold before. Ordinary strokes are a few hundred samples
 * each and are saved in about a millisecond, so neither bound is reached by
 * drawing; they only stop a stalled project from collecting ink without end. */
internal const val MAX_UNSAVED_SAMPLES = 4L * MAX_STROKE_SAMPLES

/** What a pen-down is checked against. Every field is a state in which the
 * pen must not act, or for [listsBusy] must not act yet for anything but ink. */
internal data class PenDownState(
    /** The capture stream panel has the pen, or its annotation is being saved. */
    val streamOwnsInput: Boolean,
    /** The editor is closing, or a project is being opened, closed, imported,
     * exported or connected. */
    val busy: Boolean,
    /** Something other than the canvas has the screen or the pen: another
     * screen, a dialog, sheet, menu or message, a workflow or frame panel, a
     * document operation, or an object rotation being saved. */
    val modal: Boolean,
    /** The instruction or marker list is being read again, as it is after
     * every saved edit. Anything that edits what is shown waits for it. A new
     * ink stroke reads neither list and is taken regardless. */
    val listsBusy: Boolean,
    /** The page picture the pen would draw on is not shown (yet). */
    val pictureMissing: Boolean,
    /** Strokes in flight, and the samples they hold together. */
    val strokesInFlight: Int,
    val unsavedSamples: Long,
)

/** Why a pen-down is not acted on, or null when it is. [ink]: it asks for a
 * plain ink stroke ([asksForInk]). Saving and showing earlier strokes is not
 * among the reasons: only the two bounds on what is still unsaved are, and
 * the caller shows a message for those. */
internal fun penDownRefusal(state: PenDownState, ink: Boolean): PenRefusal? = when {
    state.streamOwnsInput -> PenRefusal.Stream
    state.busy -> PenRefusal.Busy
    state.modal -> PenRefusal.Modal
    !ink && state.listsBusy -> PenRefusal.Lists
    state.pictureMissing -> PenRefusal.Loading
    state.strokesInFlight >= MAX_STROKES_IN_FLIGHT || state.unsavedSamples >= MAX_UNSAVED_SAMPLES -> PenRefusal.Backlog
    else -> null
}

/** Runs [ink] at once and [save] under [lock], in the order the calls were
 * made: the place in the lock's queue is taken before [ink] first suspends.
 * Whatever else waits for the lock afterwards (a later stroke, an edit, undo,
 * closing) runs after this save, and a later stroke's ink never waits for it.
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

/** The document as it is now. The project is told which strokes [held]
 * already has and rebuilds only the others; [held] is null for a first read.
 * It has no such path for paged documents, nor for a document at the size
 * limit of a read, and then answers null: the complete read decides. */
internal suspend fun readDocument(current: WorkbenchProject, documentId: String, held: DocumentSnapshot?): DocumentSnapshot =
    (current as? WorkbenchDocumentChanges)?.documentChanges(documentId, held) ?: current.document(documentId)
