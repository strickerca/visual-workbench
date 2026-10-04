package com.visualworkbench.desktop

import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogWindow
import androidx.compose.ui.window.rememberDialogState

/** Modal window owns field focus; the editor's shortcut/follow admission also
 * checks the same synchronous controller flag before accepting remote focus. */
@Composable
internal fun AiEditorOverlay(editor: EditorController) {
    val state by editor.ai.state.collectAsState()
    if (!state.open) return
    var keys by remember { mutableStateOf(false) }
    DialogWindow(onCloseRequest = editor::hideAi, title = "AI image edit",
        state = rememberDialogState(width = 1120.dp, height = 800.dp)) {
        MaterialTheme {
            AiWorkspace(editor.ai, editor.aiService, editor.core, { keys = true }, editor::hideAi)
            if (keys) AiKeyDialog(editor.aiService, { keys = false })
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
