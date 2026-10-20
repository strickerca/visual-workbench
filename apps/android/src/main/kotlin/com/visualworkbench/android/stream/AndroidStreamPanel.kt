package com.visualworkbench.android.stream

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.visualworkbench.shared.CaptureStreamStage
import kotlinx.coroutines.delay

/** Root mounts this in its existing modal/attachment gate and owns close retry.
 * Disposing the composable pauses input/draw admission; it is not a close ACK. */
@Composable
internal fun AndroidStreamPanel(session: AndroidStreamSession, onClose: () -> Unit, modifier: Modifier = Modifier) {
    val signal by session.signal.collectAsState()
    val state by session.annotation.state.collectAsState()
    val issue by session.issue.collectAsState()
    val lossless by session.losslessDisplayed.collectAsState()
    val surfaceIssue by session.canvas.issue.collectAsState()
    val closeRequested by session.closeRequested.collectAsState()
    var confirmDegraded by remember(session) { mutableStateOf(false) }
    var confirmDiscard by remember(session) { mutableStateOf(false) }
    var confirmClose by remember(session) { mutableStateOf(false) }
    var now by remember(session) { mutableStateOf(System.currentTimeMillis()) }
    LaunchedEffect(session) { session.activate(); while (true) { delay(1000); now = System.currentTimeMillis() } }
    DisposableEffect(session) { onDispose { session.background() } }
    LaunchedEffect(closeRequested) {
        if (closeRequested) {
            if (state.frozen != null && state.saved == null) confirmClose = true
            else { session.cancelCloseRequest(); onClose() }
        }
    }
    Column(modifier) {
        val frozen = state.frozen != null
        val mode = if (frozen) "FROZEN" else if (signal.stage == CaptureStreamStage.Live) "LIVE"
            else if (signal.stage == CaptureStreamStage.Closed) "CLOSED" else "WAITING"
        Text("$mode · ${signal.sourceName ?: "Computer capture"}", Modifier.padding(horizontal = 12.dp))
        if (!signal.sourceAvailable) Text(if (frozen) "Source unavailable · frozen frame retained" else "Source unavailable",
            Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.caption)
        if (frozen) {
            val age = session.annotation.estimatedFrameAgeMs(now)
            Text((if (age == null) "Frame age unavailable" else "Frame age ≈ ${kotlin.math.floor(age / 1000).toLong()} s") +
                if (lossless) " · lossless displayed" else " · preview displayed",
                Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.caption)
        }
        AndroidView(factory = { session.canvas }, modifier = Modifier.fillMaxWidth().weight(1f))
        (surfaceIssue ?: issue ?: state.failure ?: signal.failure)?.let { Text(it, Modifier.padding(12.dp)) }
        if (state.saved != null) Text(if (state.degradedSaved) "Saved capture · permanently degraded" else "Saved capture · verified original",
            Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.caption)
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            if (issue != null && session.displayRetryAllowed) TextButton(session::retryDisplay) { Text("Retry display") }
            if (frozen) {
                if (state.draft == null && !state.editorFailed)
                    TextButton(session::openFrozenEditor, enabled = !state.preparing && signal.stage != CaptureStreamStage.Closed) { Text("Retry frozen frame") }
                TextButton({ session.save() }, enabled = state.draft != null && !state.saving && !state.editorFailed &&
                    state.saved == null && (state.lossless || state.saveRetry) && (signal.stage != CaptureStreamStage.Closed || state.saveRetry)) {
                    Text(if (state.saveRetry) "Retry same Save" else "Save capture")
                }
                if (!state.lossless && !state.saveRetry && state.saved == null && signal.stage == CaptureStreamStage.FrozenDegraded)
                    TextButton({ confirmDegraded = true }, enabled = state.draft != null && !state.saving && !state.editorFailed) { Text("Save degraded…") }
                TextButton({ if (state.saved == null) confirmDiscard = true else session.resume() },
                    enabled = !state.saving && !state.saveRetry && signal.stage != CaptureStreamStage.Closed) {
                    Text(if (state.saved == null) "Discard draft…" else "Resume live")
                }
            } else Text("Touch with the pen to freeze the displayed frame", Modifier.padding(12.dp), style = MaterialTheme.typography.caption)
            TextButton(session::requestClose, enabled = !state.saving && !state.saveRetry) { Text("Close") }
        }
    }
    if (confirmDegraded) AlertDialog(onDismissRequest = { confirmDegraded = false },
        title = { Text("Save degraded preview?") },
        text = { Text("The exact lossless original is unavailable. This saves the displayed preview and permanently marks the capture as degraded.") },
        confirmButton = { TextButton({ confirmDegraded = false; session.save(true) }) { Text("Save degraded capture") } },
        dismissButton = { TextButton({ confirmDegraded = false }) { Text("Keep waiting") } })
    if (confirmDiscard) AlertDialog(onDismissRequest = { confirmDiscard = false },
        title = { Text("Discard this annotation draft?") },
        text = { Text("This closes only the frozen draft and resumes live capture. The paired project remains open.") },
        confirmButton = { TextButton({ confirmDiscard = false; session.resume() }) { Text("Discard draft") } },
        dismissButton = { TextButton({ confirmDiscard = false }) { Text("Keep draft") } })
    if (confirmClose) AlertDialog(onDismissRequest = { confirmClose = false; session.cancelCloseRequest() },
        title = { Text("Close and discard this draft?") },
        text = { Text("The frozen annotations have not been saved. Closing discards only this temporary draft.") },
        confirmButton = { TextButton({ confirmClose = false; session.cancelCloseRequest(); onClose() }) { Text("Discard and close") } },
        dismissButton = { TextButton({ confirmClose = false; session.cancelCloseRequest() }) { Text("Keep draft") } })
}
