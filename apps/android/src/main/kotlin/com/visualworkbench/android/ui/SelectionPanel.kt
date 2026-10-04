package com.visualworkbench.android.ui

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.android.editor.SelectionController
import com.visualworkbench.shared.*

@Composable
internal fun SelectionControls(controller: SelectionController, selectedObject: String?,
    onSave: (SelectionSaveTicket) -> Unit, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    var open by remember { mutableStateOf(false) }
    Surface(modifier, elevation = 4.dp) {
        Row(Modifier.horizontalScroll(rememberScrollState())) {
            TextButton({ open = true }, Modifier.heightIn(min = 48.dp)) { Text(if (state.active) "Selection · ${state.tool.name}" else "Select pixels") }
            if (state.active) TextButton({ controller.interaction.active(false) }, Modifier.heightIn(min = 48.dp)) { Text("Return to drawing") }
            if (state.busy) TextButton(controller::cancel, Modifier.heightIn(min = 48.dp)) { Text("Cancel selection") }
        }
    }
    if (open) SelectionPanel(controller, selectedObject, { ticket -> open = false; onSave(ticket) }, { open = false })
}

@Composable
internal fun SelectionPanel(controller: SelectionController, selectedObject: String?,
    onSave: (SelectionSaveTicket) -> Unit, onDismiss: () -> Unit) {
    val state by controller.state.collectAsState(); val saved by controller.saved.collectAsState()
    var crop by remember { mutableStateOf(false) }
    val actions = controller.interaction
    AlertDialog(onDismissRequest = onDismiss, title = { Text("Pixel selection") }, text = {
        Column(Modifier.heightIn(max = 600.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Choose a tool, then draw on the image. Magenta shows saved coverage; the gesture outline is a guide.")
            SelectionTool.entries.forEach { tool ->
                OutlinedButton({ actions.tool(tool) }, Modifier.fillMaxWidth().heightIn(min = 48.dp), enabled = !state.busy) {
                    Text((if (state.active && state.tool == tool) "• " else "") + when (tool) {
                        SelectionTool.Rectangle -> "Rectangle"; SelectionTool.Lasso -> "Lasso"
                        SelectionTool.Paint -> "Paint selection"; SelectionTool.MaskEraser -> "Erase mask"
                    })
                }
            }
            Row(Modifier.horizontalScroll(rememberScrollState())) {
                SelectionCombine.entries.forEach { mode -> TextButton({ actions.combine(mode) }, Modifier.heightIn(min = 48.dp), enabled = !state.busy && state.tool != SelectionTool.MaskEraser) { Text((if (state.combine == mode) "• " else "") + mode.name) } }
            }
            if (state.tool == SelectionTool.Paint || state.tool == SelectionTool.MaskEraser) {
                Text("Radius ${state.radius.toInt()} document pixels")
                Slider(state.radius.toFloat(), { actions.brush(it.toDouble()) }, valueRange = .5f..128f, enabled = !state.busy)
                Text("Strength ${(state.opacity.toInt() * 100 + 127) / 255}%")
                Slider(state.opacity.toFloat(), { actions.brush(state.radius, it.toInt().coerceIn(0, 255).toUByte()) }, valueRange = 0f..255f, enabled = !state.busy)
            }
            Row { TextButton(actions::newSelection, Modifier.heightIn(min = 48.dp), enabled = !state.busy) { Text("New selection") }
                TextButton({ selectedObject?.let(actions::useObject) }, Modifier.heightIn(min = 48.dp), enabled = selectedObject != null && !state.busy) { Text("Use selected mask") } }
            if (state.selection != null) {
                Text("Refine · ${state.refinementRadius} pixels")
                Slider(state.refinementRadius.toFloat(), { actions.refinementRadius(it.toInt().coerceIn(1, 64).toUInt()) }, valueRange = 1f..64f, steps = 62, enabled = state.canRefine)
                Row(Modifier.horizontalScroll(rememberScrollState())) {
                    SelectionRefinement.entries.forEach { action -> TextButton({ actions.refine(action) }, Modifier.heightIn(min = 48.dp), enabled = state.canRefine) { Text(action.name) } }
                }
                Row { Checkbox(crop, { crop = it }); Text("Crop to selection bounds", Modifier.padding(top = 12.dp)) }
                SelectionExportKind.entries.forEach { kind ->
                    OutlinedButton({ controller.ticket(kind, crop)?.let(onSave) }, Modifier.fillMaxWidth().heightIn(min = 48.dp),
                        enabled = controller.ticket(kind, crop) != null && !saved.saving) { Text(if (kind == SelectionExportKind.Mask) "Save mask PNG" else "Save original cutout PNG") }
                }
                Text("Cutout preserves the original image's bit depth and color profile.", style = MaterialTheme.typography.caption)
            }
            if (state.preview?.limitedToViewport == true) Text("The preview shows the central 1024 px window. Zoom or pan to inspect; exports use the full selection.")
            if (state.loading || state.busy || saved.saving) LinearProgressIndicator(Modifier.fillMaxWidth())
            state.message?.let { Text(it) }; saved.message?.let { Text(it) }
            if (state.busy || saved.saving) TextButton(controller::cancel, Modifier.heightIn(min = 48.dp)) { Text("Cancel operation") }
        }
    }, confirmButton = { TextButton(onDismiss, Modifier.heightIn(min = 48.dp)) { Text("Done") } })
}
