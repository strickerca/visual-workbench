package com.visualworkbench.android.stream

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.visualworkbench.android.editor.EditorTool
import com.visualworkbench.shared.CaptureStreamStage

@Composable
internal fun StreamPanel(owner: AndroidStreamController, close: () -> Unit) {
    val state by owner.state.collectAsState()
    val annotation by owner.annotation.state.collectAsState()
    var discard by remember { mutableStateOf<StreamDiscardAction?>(null) }
    var degraded by remember { mutableStateOf(false) }
    var ageNow by remember { mutableStateOf(System.currentTimeMillis()) }
    LaunchedEffect(annotation.frozen?.draftId) {
        if(annotation.frozen!=null)while(true){ageNow=System.currentTimeMillis();kotlinx.coroutines.delay(500)}
    }
    fun requestClose() { if (annotation.frozen != null && annotation.saved == null) discard = StreamDiscardAction.Close else close() }
    Dialog(onDismissRequest = ::requestClose, properties = DialogProperties(usePlatformDefaultWidth = false, dismissOnClickOutside = false)) {
        Surface(Modifier.fillMaxSize().safeDrawingPadding()) {
            Column {
                Row(Modifier.fillMaxWidth().padding(8.dp)) {
                    Text("Computer capture", Modifier.weight(1f), style = MaterialTheme.typography.h6)
                    TextButton(onClick = ::requestClose, enabled = !state.closing) { Text("Close stream") }
                }
                val label = when {
                    annotation.saved != null -> "Saved in the paired project${if (annotation.degradedSaved) " · degraded preview" else " · exact original"} · revision ${annotation.saved!!.revision.hostSeq}"
                    annotation.lossless -> "Exact original verified · annotations remain private until Save"
                    state.stage == CaptureStreamStage.FrozenDegraded -> "Lossless frame unavailable · preview annotations are temporary until an explicit degraded Save"
                    annotation.frozen != null -> "Frozen preview draft · waiting for this frame's lossless original"
                    state.stage == CaptureStreamStage.AwaitingSource -> "Choose and approve a window, monitor or region on the computer."
                    state.stage == CaptureStreamStage.SourceClosed -> "The approved source closed. No new frames can arrive."
                    else -> "Live preview · pen contact freezes the frame currently drawn"
                }
                if(annotation.frozen!=null) {
                    val age=owner.annotation.estimatedFrameAgeMs(ageNow)
                    Text(if(age!=null) "FROZEN · estimated frame age ${(age/1000).toLong()} s (link clock estimate)"
                        else "FROZEN · frame age unavailable without a link clock estimate",Modifier.padding(horizontal=16.dp),style=MaterialTheme.typography.caption)
                }
                Text(label, Modifier.padding(horizontal = 16.dp), style = MaterialTheme.typography.caption)
                if (state.busy || state.closing) LinearProgressIndicator(Modifier.fillMaxWidth())
                val editor = state.child
                if (editor != null) {
                    Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp)) {
                        for (tool in listOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Line, EditorTool.Arrow, EditorTool.Rectangle, EditorTool.Ellipse, EditorTool.Select)) {
                            TextButton(onClick = { editor.chooseTool(tool) }, enabled = !owner.inputSealed && !state.busy) { Text(if (editor.tool == tool) "• ${tool.label}" else tool.label) }
                        }
                        TextButton(onClick = editor::undo, enabled = !owner.inputSealed && !state.busy && editor.pending == 0) { Text("Undo") }
                        TextButton(onClick = editor::redo, enabled = !owner.inputSealed && !state.busy && editor.pending == 0) { Text("Redo") }
                    }
                }
                Box(Modifier.weight(1f).fillMaxWidth()) { AndroidView(factory = { StreamCanvasView(it, owner) }, modifier = Modifier.fillMaxSize()) }
                state.failure?.let { Text(it, Modifier.padding(12.dp), color = MaterialTheme.colors.error) }
                editor?.message?.let { Text(it, Modifier.padding(horizontal = 12.dp), color = MaterialTheme.colors.error) }
                Row(Modifier.fillMaxWidth().padding(8.dp)) {
                    if (annotation.frozen != null) {
                        Button(onClick = { owner.save() }, enabled = (annotation.lossless || annotation.saveRetry) && annotation.saved == null && !state.busy && !state.closing && editor?.pending == 0) {
                            Text(if (annotation.saveRetry) "Retry exact Save" else "Save lossless capture")
                        }
                        if (!annotation.lossless && state.stage == CaptureStreamStage.FrozenDegraded && annotation.saved == null) {
                            TextButton(onClick = { degraded = true }, enabled = !state.busy && !state.closing && editor?.pending == 0) { Text("Save degraded preview…") }
                        }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = { if (annotation.saved != null) owner.resume() else discard = StreamDiscardAction.Resume }, enabled = !state.busy && !state.closing) { Text("Resume live") }
                    }
                    if (annotation.frozen == null && state.stage == CaptureStreamStage.AwaitingSource) {
                        OutlinedButton(onClick = owner::requestComputerRegion, enabled = !state.busy && !state.closing) { Text("Request computer region picker") }
                    }
                    if (state.busy) TextButton(onClick = owner::cancelAction) { Text("Cancel operation") }
                }
            }
        }
        if (discard != null) AlertDialog(onDismissRequest = { discard = null },
            title = { Text("Discard this temporary annotation draft?") },
            text = { Text("Unsaved annotations will be removed only after pending native work settles. The paired project and its existing original stay open. A preview will never be saved as a lossless original.") },
            confirmButton = { TextButton(onClick = { val action = discard; discard = null; if (action == StreamDiscardAction.Close) close() else owner.resume() }) { Text(if (discard == StreamDiscardAction.Close) "Discard and close" else "Discard and resume") } },
            dismissButton = { TextButton(onClick = { discard = null }) { Text("Keep draft") } })
        if (degraded) AlertDialog(onDismissRequest = { degraded = false },
            title = { Text("Save this degraded preview?") },
            text = { Text("This frame's lossless original is unavailable. Save keeps the exact displayed preview and your annotations, with permanent degraded capture metadata. It cannot restore details lost by JPEG compression.") },
            confirmButton = { TextButton(onClick = { degraded = false; owner.save(true) }) { Text("Save flagged degraded preview") } },
            dismissButton = { TextButton(onClick = { degraded = false }) { Text("Keep waiting") } })
    }
}

internal enum class StreamDiscardAction { Close, Resume }
