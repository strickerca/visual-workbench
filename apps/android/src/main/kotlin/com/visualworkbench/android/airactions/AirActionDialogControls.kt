package com.visualworkbench.android.airactions

import android.view.KeyEvent
import android.view.Window
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.DialogWindowProvider
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.shared.RemoteEditorPalette
import com.visualworkbench.shared.WorkbenchRemoteEdit

/** Borrows the actual dialog Window and the existing controller's exact palette.
 * It adds the same pre-fallback key callback as the Activity, with no new input
 * owner, grant, remote controller or keyboard injection. */
@Composable
internal fun AirActionDialogControls(editor: EditorController, remote: WorkbenchRemoteEdit,
    palette: RemoteEditorPalette, blocked: Boolean) {
    val view = LocalView.current
    val window = (view.parent as? DialogWindowProvider)?.window
    val latestBlocked = rememberUpdatedState(blocked)
    DisposableEffect(editor, remote, palette, window) {
        val previous = window?.callback
        if (window == null || previous == null) onDispose { }
        else {
            editor.attachAirRemotePalette(remote, palette)
            val callback = object : Window.Callback by previous {
                override fun dispatchKeyEvent(event: KeyEvent): Boolean {
                    if (latestBlocked.value) editor.airRemoteFocus(remote, palette, false)
                    return editor.airRemoteKey(remote, palette, event) || previous.dispatchKeyEvent(event)
                }
                override fun onWindowFocusChanged(hasFocus: Boolean) {
                    editor.airRemoteFocus(remote, palette, hasFocus && !latestBlocked.value)
                    previous.onWindowFocusChanged(hasFocus)
                }
            }
            window.callback = callback
            editor.airRemoteFocus(remote, palette, window.decorView.hasWindowFocus() && !latestBlocked.value)
            onDispose {
                if (window.callback === callback) window.callback = previous
                editor.detachAirRemotePalette(remote, palette)
            }
        }
    }
    SideEffect { editor.airRemoteFocus(remote, palette, window?.decorView?.hasWindowFocus() == true && !blocked) }
    AirActionControls(editor)
}
