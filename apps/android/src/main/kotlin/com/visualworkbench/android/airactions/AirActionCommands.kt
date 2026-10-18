package com.visualworkbench.android.airactions

import com.visualworkbench.shared.AirActionCommand
import com.visualworkbench.shared.RemoteEditorAction

/** Exact existing palette actions only. A fit without exact restore or a guessed
 * tool order must stay unavailable in the remote context. */
internal fun remoteAirAction(command: AirActionCommand): RemoteEditorAction? = when (command) {
    AirActionCommand.Undo -> RemoteEditorAction.Undo
    AirActionCommand.Redo -> RemoteEditorAction.Redo
    else -> null
}
