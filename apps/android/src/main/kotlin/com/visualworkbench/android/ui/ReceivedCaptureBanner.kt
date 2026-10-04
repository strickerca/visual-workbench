package com.visualworkbench.android.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.android.editor.EditorController

@Composable
internal fun ReceivedCaptureBanner(editor: EditorController) {
    val received by editor.captureArrivals.state.collectAsState()
    val failure by editor.captureArrivals.failure.collectAsState()
    if (failure != null) Text(checkNotNull(failure), Modifier.padding(8.dp))
    val value = received ?: return
    val instructions by editor.instructionEditor.state.collectAsState()
    val semantics by editor.semanticEditor.state.collectAsState()
    val selections by editor.selections.state.collectAsState()
    val erasers by editor.erasers.state.collectAsState()
    val ai by editor.ai.state.collectAsState()
    val ownersReady = !instructions.busy && !semantics.busy && !selections.active && !erasers.active && !ai.open
    val ready = ownersReady && editor.scene != null && value.originalVerified && editor.canOpenReceivedCapture()
    val automatically = ready && editor.autoOpenReceivedCapture(value)
    LaunchedEffect(value, automatically, editor.captureAdmissionVersion) { if (automatically) editor.openReceivedCapture(value) }
    Surface(color = MaterialTheme.colors.surface, elevation = 4.dp) {
        Row(Modifier.fillMaxWidth().padding(8.dp)) {
            Text(if (value.originalVerified) "New lossless capture saved. Finish the current draft or gesture to open it."
                else "Receiving the capture original…", Modifier.weight(1f))
            TextButton(onClick = { editor.openReceivedCapture(value) }, enabled = ready) { Text("Open capture") }
        }
    }
}
