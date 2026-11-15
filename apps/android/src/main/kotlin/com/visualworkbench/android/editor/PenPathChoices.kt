package com.visualworkbench.android.editor

import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.EraserInputRoute
import com.visualworkbench.shared.StrokeOptions
import com.visualworkbench.shared.WorkbenchDeferredStrokes
import com.visualworkbench.shared.WorkbenchDocumentChanges
import com.visualworkbench.shared.WorkbenchProject
import com.visualworkbench.shared.WorkbenchStroke

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

/** Begins the native stroke. With [lockFree] it begins without reading the
 * project when the core offers that; it does not for a highlighter and then
 * answers null, which falls back to the original call. */
internal suspend fun beginInkStroke(current: WorkbenchProject, options: StrokeOptions, lockFree: Boolean): WorkbenchStroke =
    (if (lockFree) (current as? WorkbenchDeferredStrokes)?.beginStrokeDeferred(options) else null)
        ?: current.beginStroke(options)

private val INK_TOOLS = setOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker)

/** Whether a pen-down may pass while the instruction and marker lists are
 * read again after a save. Only a plain ink stroke may, and only with the
 * switch on; [route] is asked last and not at all with the switch off. */
internal fun inkPassesListRefresh(switchOn: Boolean, tool: EditorTool, assistedCollecting: Boolean,
    textTool: Boolean, regionTool: Boolean, route: () -> EraserInputRoute): Boolean =
    switchOn && tool in INK_TOOLS && !assistedCollecting && !textTool && !regionTool && route() == EraserInputRoute.Editor

/** One ink frame: true when the whole scene has to be drawn, as it always was.
 * [inkOnly] draws the kept picture with only the new stroke over it and says
 * whether it could; it is asked only when the scene carries the switch. */
internal inline fun wholeSceneNeeded(keepDryPicture: Boolean, inkOnly: () -> Boolean): Boolean =
    !(keepDryPicture && inkOnly())
