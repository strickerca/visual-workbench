package com.visualworkbench.android.editor

import android.content.Context
import android.content.ContextWrapper
import androidx.compose.material.*
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Modifier
import com.visualworkbench.android.ui.AgentCaptureIndicator
import com.visualworkbench.android.ui.ReceivedCaptureBanner
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.LifecycleOwner
import com.visualworkbench.shared.AndroidProviderKeyStore

private fun lifecycleOwner(context: Context): LifecycleOwner? {
    var value = context
    repeat(16) {
        if (value is LifecycleOwner) return value as LifecycleOwner
        val next = (value as? ContextWrapper)?.baseContext ?: return null
        if (next === value) return null
        value = next
    }
    return null
}

@Composable
internal fun AiEditorOverlay(editor: EditorController) {
    val state by editor.ai.state.collectAsState()
    if (!state.open) return
    val lifecycle = lifecycleOwner(LocalContext.current)?.lifecycle
    var keys by remember { mutableStateOf<AndroidProviderKeyStore?>(null) }
    var keyFailure by remember { mutableStateOf(false) }
    Dialog(onDismissRequest = editor::hideAi, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Column(Modifier.fillMaxSize()) {
            val captureStatus by editor.agentCapture.state.collectAsState()
            AgentCaptureIndicator(captureStatus)
            ReceivedCaptureBanner(editor)
        AiWorkspace(editor.ai, editor.aiService, editor.core, {
            try {
                if (lifecycle == null) keyFailure = true else keys = editor.aiKeyStore()
            } catch (_: Exception) { keyFailure = true }
        }, editor::hideAi, Modifier.weight(1f))
        }
        if (keyFailure) AlertDialog(onDismissRequest = { keyFailure = false },
            title = { Text("Protected settings unavailable") },
            text = { Text("Reopen the app and try API key settings again. No key was saved or removed.") },
            confirmButton = { TextButton({ keyFailure = false }) { Text("Close") } })
        val store = keys
        if (store != null && lifecycle != null) ProviderKeyDialog(store, lifecycle) {
            keys = null; editor.aiService.activate()
        }
    }
}

@Composable
internal fun AiResultStatus(editor: EditorController) {
    val state by editor.aiResults.state.collectAsState()
    if (state.loading) Text("Loading saved Result pixels…", style = MaterialTheme.typography.caption)
    state.message?.let { text ->
        Text(text, style = MaterialTheme.typography.caption)
        TextButton(onClick = editor.aiResults::retry) { Text("Retry Result display") }
    }
}
