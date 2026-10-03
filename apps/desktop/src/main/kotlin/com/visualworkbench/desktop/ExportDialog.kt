package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable
internal fun ExportDialog(controller: EditorController, state: EditorState, save: (ExportRequest) -> Unit) {
    val settings = state.exportSettings
    val request = remember(state.document, state.projectEpoch, settings, state.selected, state.view.camera) {
        runCatching { captureExport(state, controller.core) }.getOrNull()
    }
    val enabled = !state.busy && request != null
    AlertDialog(
        onDismissRequest = { if (state.busy) controller.cancelTransfer() else controller.dismissExport() },
        title = { Text("Export image") },
        text = {
            Column(Modifier.width(650.dp).heightIn(max = 620.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                request?.let {
                    Text("${it.revision} · ${it.width} × ${it.height} px", style = MaterialTheme.typography.subtitle1)
                } ?: Text("Select a region that intersects the image, or choose Full image.")
                Text("Exports use the original image at document resolution. Region exports use rectangular pixel bounds.", style = MaterialTheme.typography.caption)
                Row(horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                    Choice("Content", if (settings.marked) "Marked" else "Clean", listOf("Marked", "Clean"), !state.busy) {
                        controller.exportSettings(settings.copy(marked = it == "Marked"))
                    }
                    Choice("Area", settings.area.label, ExportArea.entries.map { it.label }, !state.busy) { label ->
                        controller.exportSettings(settings.copy(area = ExportArea.entries.first { it.label == label }))
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                    Choice("Format", settings.encoding.label, ExportEncoding.entries.map { it.label }, !state.busy) { label ->
                        controller.exportSettings(settings.copy(encoding = ExportEncoding.entries.first { it.label == label }))
                    }
                    Choice("Transparency", settings.matte.label, ExportMatte.entries.map { it.label }, !state.busy) { label ->
                        controller.exportSettings(settings.copy(matte = ExportMatte.entries.first { it.label == label }))
                    }
                }
                if (settings.encoding == ExportEncoding.Jpeg || settings.encoding == ExportEncoding.WebpLossy) {
                    Text("Quality ${settings.quality}")
                    Slider(settings.quality.toFloat(), { controller.exportSettings(settings.copy(quality = it.toInt())) },
                        enabled = !state.busy, valueRange = 1f..100f, steps = 98)
                }
                Option("Convert this export to sRGB", settings.convertToSrgb, !state.busy) { controller.exportSettings(settings.copy(convertToSrgb = it)) }
                Option("Treat untagged source colors as sRGB", settings.assumeUntaggedSrgb, !state.busy) { controller.exportSettings(settings.copy(assumeUntaggedSrgb = it)) }
                Option("Allow 16-bit samples to be reduced for this export", settings.allowDepthReduction, !state.busy) { controller.exportSettings(settings.copy(allowDepthReduction = it)) }
                Text("WebP: up to 16,383 px per side. JPEG: up to 65,535 px per side. All formats also have pixel, memory, encoded-byte and scratch limits; a refusal keeps the source unchanged.", style = MaterialTheme.typography.caption)
                if (settings.encoding.png) {
                    Divider()
                    Text("Windows clipboard companion", style = MaterialTheme.typography.subtitle2)
                    Text("Copy publishes the exact PNG plus an 8-bit DIBV5 image. PNG file drag preserves the chosen PNG depth.", style = MaterialTheme.typography.caption)
                    Option("Allow depth reduction for the DIBV5 companion", settings.allowDibDepthReduction, !state.busy) { controller.exportSettings(settings.copy(allowDibDepthReduction = it)) }
                    Option("Treat an untagged DIBV5 companion as sRGB", settings.assumeDibSrgb, !state.busy) { controller.exportSettings(settings.copy(assumeDibSrgb = it)) }
                }
                state.exportCheck?.takeIf { it.request == request }?.result?.let { check ->
                    Text("Preflight: ${check.width} × ${check.height} · ${check.outputBitDepth} bit · estimated ${check.estimatedPeakBytes / (1024uL * 1024uL)} MiB${if (check.buffered) " · buffered codec" else " · strip encoder"}")
                    if (check.requiresRenderValidation) Text("Marked content is validated again when the completed file is prepared.", style = MaterialTheme.typography.caption)
                }
                if (state.exportReady) state.exportReceipt?.let { Text("EXPORT READY · ${it.revision} · ${it.bytes} bytes", color = MaterialTheme.colors.primary) }
                state.message?.let { Text(it, style = MaterialTheme.typography.body2) }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton({ request?.let(controller::checkExport) }, enabled = enabled) { Text("Check limits") }
                    OutlinedButton({ request?.let(controller::prepareExportFile) }, enabled = enabled) { Text("Prepare") }
                    Button({ request?.let(save) }, enabled = enabled) { Text("Save file…") }
                }
                if (settings.encoding.png) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedButton({ request?.let(controller::copyExport) }, enabled = enabled) { Text("Copy PNG + DIBV5") }
                        OutlinedButton({ request?.let(controller::prepareDragExport) }, enabled = enabled) { Text("Prepare file drag") }
                    }
                    Text("Preparing a drag returns to the canvas. Use the Drag prepared PNG button on the transfer shelf.", style = MaterialTheme.typography.caption)
                }
                Text("Copy replaces the clipboard only when you press Copy. Drag sends a PNG file to the app you choose; review it there before sending.", style = MaterialTheme.typography.caption)
                if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            }
        },
        confirmButton = { TextButton({ if (state.busy) controller.cancelTransfer() else controller.dismissExport() }) { Text(if (state.busy) "Cancel transfer" else "Close") } },
    )
}

@Composable
internal fun PasteDialog(controller: EditorController, state: EditorState) {
    var assumeSrgb by remember { mutableStateOf(false) }
    val epoch = remember { state.projectEpoch }
    AlertDialog(onDismissRequest = controller::dismissPaste, title = { Text("Paste image") }, text = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Import a PNG or DIB image into a new project. PNG is preferred when available.")
            Text("Clipboard images have a 16 MP limit; PNG data is limited to 32 MiB. Use File > Import for larger originals up to 50 MP.", style = MaterialTheme.typography.caption)
            Option("Treat untagged DIB colors as sRGB", assumeSrgb, true) { assumeSrgb = it }
        }
    }, confirmButton = { Button({ controller.pasteImage(assumeSrgb, epoch) }) { Text("Paste image") } },
        dismissButton = { TextButton(controller::dismissPaste) { Text("Cancel") } })
}

@Composable
private fun Choice(label: String, selected: String, values: List<String>, enabled: Boolean, change: (String) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    Column {
        Text(label, style = MaterialTheme.typography.caption)
        Box {
            OutlinedButton({ expanded = true }, enabled = enabled) { Text(selected) }
            DropdownMenu(expanded, { expanded = false }) {
                values.forEach { value -> DropdownMenuItem({ expanded = false; change(value) }) { Text(value) } }
            }
        }
    }
}
@Composable
private fun Option(label: String, selected: Boolean, enabled: Boolean, change: (Boolean) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(selected, change, enabled = enabled)
        Text(label, style = MaterialTheme.typography.body2)
    }
}
