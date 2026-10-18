package com.visualworkbench.android.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import com.visualworkbench.shared.outputWidth
import com.visualworkbench.shared.outputHeight
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.android.editor.*
import kotlin.math.roundToInt

@Composable
internal fun ExportDialog(editor: EditorController, onDismiss: () -> Unit, onSave: () -> Unit, onShare: () -> Unit) {
    val choice = editor.exportSelection
    val output by editor.imageOutputs.state.collectAsState()
    val idle = !editor.busy && editor.pending == 0
    val committed = editor.document?.let(editor.imageOutputs::exportSnapshot)
    LaunchedEffect(editor.document?.render?.revision?.stateHash) { editor.imageOutputs.requestExportContext() }
    val enabled = idle && committed != null && !output.loading
    val checked = editor.exportPreflight
    AlertDialog(onDismissRequest = { if (idle) onDismiss() }, title = { Text("Export image") }, text = {
        Column(Modifier.heightIn(max = 560.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            committed?.let { Text("Committed output: ${it.outputWidth} × ${it.outputHeight} px · original ${it.sourceWidth} × ${it.sourceHeight} px") }
                ?: Text(if(output.loading)"Loading committed output settings…" else output.issue ?: "Output settings are unavailable. Close and reopen Export to retry.")
            Toggle("Include annotations (marked)", choice.marked, enabled) { editor.configureExport(choice.copy(marked = it)) }
            Toggle("Crop to current view bounds", choice.viewOnly, enabled) { editor.configureExport(choice.copy(viewOnly = it)) }
            if (choice.viewOnly) Text("Maps the current view bounds through the committed crop, quarter-turn and size, then clips to output pixels. Camera rotation encloses a rectangle; it does not change the saved output frame.", style = MaterialTheme.typography.caption)
            Text("Format", style = MaterialTheme.typography.subtitle2)
            ExportEncoding.entries.forEach { encoding ->
                TextButton(onClick = { editor.configureExport(choice.copy(encoding = encoding)) }, enabled = enabled,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text(if (choice.encoding == encoding) "✓ ${encoding.label}" else encoding.label) }
            }
            if (choice.encoding in listOf(ExportEncoding.Jpeg, ExportEncoding.WebpLossy)) {
                Text("Quality ${choice.quality}")
                Slider(choice.quality.toFloat(), { editor.configureExport(choice.copy(quality = it.roundToInt())) }, enabled = enabled,
                    valueRange = 1f..100f, steps = 98, modifier = Modifier.heightIn(min = 48.dp))
            }
            if (choice.encoding == ExportEncoding.Jpeg) Toggle("Flatten transparency onto white", choice.whiteMatte, enabled) { editor.configureExport(choice.copy(whiteMatte = it)) }
            if ((editor.document?.bitDepth ?: 8u) > 8u && choice.encoding != ExportEncoding.Png16)
                Toggle("Allow reduction to 8-bit output", choice.allowDepthReduction, enabled) { editor.configureExport(choice.copy(allowDepthReduction = it)) }
            Toggle("Convert exported colors to sRGB", choice.convertToSrgb, enabled) { editor.configureExport(choice.copy(convertToSrgb = it)) }
            Text("The original stays unchanged. JPEG/WebP may require more memory than PNG. Color conversion and depth reduction apply only to this export.", style = MaterialTheme.typography.caption)
            OutlinedButton(onClick = editor::prepareExport, enabled = enabled, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Check export") }
            checked?.let {
                Text("${it.width} × ${it.height} px · ${it.outputBitDepth}-bit · revision ${it.revision.hostSeq}")
                Text("Estimated workspace: ${(it.estimatedPeakBytes / (1024uL * 1024uL))} MiB. ${if (it.buffered) "Buffered encoding." else "Bounded PNG streaming."}", style = MaterialTheme.typography.caption)
                if (it.requiresRenderValidation) Text("Marks and result assets are checked again during rendering.", style = MaterialTheme.typography.caption)
            }
            editor.transferLabel?.let { Text(it); LinearProgressIndicator(Modifier.fillMaxWidth()); TextButton(onClick = editor::cancelTransfer, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel check") } }
            // The snackbar is behind this modal; show typed refusal text here as well.
            editor.message?.let { Text(it, color = MaterialTheme.colors.secondary) }
        }
    }, confirmButton = {
        Row {
            TextButton(onClick = onShare, enabled = enabled && checked != null && choice.encoding in listOf(ExportEncoding.Png8, ExportEncoding.Png16), modifier = Modifier.heightIn(min = 48.dp)) { Text("Share PNG") }
            TextButton(onClick = onSave, enabled = enabled && checked != null, modifier = Modifier.heightIn(min = 48.dp)) { Text("Save file") }
        }
    }, dismissButton = { TextButton(onClick = onDismiss, enabled = idle, modifier = Modifier.heightIn(min = 48.dp)) { Text("Close") } })
}

@Composable
private fun Toggle(label: String, checked: Boolean, enabled: Boolean, change: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
        Checkbox(checked, change, enabled = enabled)
        Text(label, Modifier.weight(1f))
    }
}
