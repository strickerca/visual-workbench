package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable
internal fun SelectionPanel(controller: SelectionController, selectedObject: String?,
    onSave: (SelectionSaveTicket) -> Unit, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    val saving by controller.saved.collectAsState()
    var crop by remember { mutableStateOf(false) }
    val actions = controller.interaction
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row { Checkbox(state.active, { actions.active(it) }); Text("Selection tools", Modifier.padding(top = 12.dp)) }
        if (state.active) {
            Text("Original pixels · one saved step per gesture", style = MaterialTheme.typography.caption)
            SelectionTool.entries.forEach { tool ->
                OutlinedButton({ actions.tool(tool) }, Modifier.fillMaxWidth(), enabled = !state.busy) {
                    Text((if (state.tool == tool) "• " else "") + when (tool) {
                        SelectionTool.Rectangle -> "Rectangle"; SelectionTool.Lasso -> "Lasso"
                        SelectionTool.Paint -> "Paint selection"; SelectionTool.MaskEraser -> "Erase mask"
                    })
                }
            }
            Text("Combine", style = MaterialTheme.typography.caption)
            SelectionCombine.entries.forEach { mode ->
                TextButton({ actions.combine(mode) }, enabled = !state.busy && state.tool != SelectionTool.MaskEraser) {
                    Text((if (state.combine == mode) "• " else "") + mode.name)
                }
            }
            if (state.tool == SelectionTool.Paint || state.tool == SelectionTool.MaskEraser) {
                Text("Radius ${state.radius.toInt()} px")
                Slider(state.radius.toFloat(), { actions.brush(it.toDouble()) }, valueRange = .5f..128f, enabled = !state.busy)
                Text("Strength ${(state.opacity.toInt() * 100 + 127) / 255}%")
                Slider(state.opacity.toFloat(), { actions.brush(state.radius, it.toInt().coerceIn(0, 255).toUByte()) }, valueRange = 0f..255f, enabled = !state.busy)
            }
            TextButton(actions::newSelection, enabled = !state.busy) { Text("New selection") }
            TextButton({ selectedObject?.let(actions::useObject) }, enabled = selectedObject != null && !state.busy) { Text("Use selected mask") }
            if (state.selection != null) {
                Text("Refine · ${state.refinementRadius} px")
                Slider(state.refinementRadius.toFloat(), { actions.refinementRadius(it.toInt().coerceIn(1, 64).toUInt()) }, valueRange = 1f..64f, steps = 62, enabled = state.canRefine)
                SelectionRefinement.entries.forEach { value -> TextButton({ actions.refine(value) }, enabled = state.canRefine) { Text(value.name) } }
                Row { Checkbox(crop, { crop = it }); Text("Crop to selection bounds", Modifier.padding(top = 12.dp)) }
                for (kind in SelectionExportKind.entries) {
                    OutlinedButton({ controller.ticket(kind, crop)?.let(onSave) }, Modifier.fillMaxWidth(),
                        enabled = controller.ticket(kind, crop) != null && !saving.saving) { Text(if (kind == SelectionExportKind.Mask) "Export mask PNG" else "Export cutout PNG") }
                }
                Text("Cutout uses the original image, preserving its depth and color profile.", style = MaterialTheme.typography.caption)
            }
            if (state.preview?.limitedToViewport == true) Text("Preview shows the central 1024 px window. Zoom or pan to inspect; export uses the entire selection.", style = MaterialTheme.typography.caption)
            if (state.loading || state.busy || saving.saving) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (state.busy || saving.saving) TextButton(controller::cancel) { Text("Cancel operation") }
            state.message?.let { Text(it, style = MaterialTheme.typography.caption) }
            saving.message?.let { Text(it, style = MaterialTheme.typography.caption) }
        }
    }
}
