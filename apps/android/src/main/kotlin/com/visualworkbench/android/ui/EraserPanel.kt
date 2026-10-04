package com.visualworkbench.android.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.android.editor.EraserController
import com.visualworkbench.shared.*

@Composable
internal fun EraserControls(controller: EraserController, choose: (EraserChoice) -> Unit,
    drawing: () -> Unit, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    val selection by controller.tools.selection.state.collectAsState()
    val mask = selection.active && selection.tool == SelectionTool.MaskEraser
    val active = state.active && !selection.active
    var expanded by remember { mutableStateOf(false) }
    val busy = state.busy || selection.busy
    Column(modifier.padding(horizontal = 8.dp)) {
        Row {
            Box {
                TextButton({ expanded = true }, enabled = !busy) { Text(if (mask) "Mask eraser" else if (active) "${state.mode.name} eraser" else "Erasers") }
                DropdownMenu(expanded, { expanded = false }) {
                    EraserChoice.entries.forEach { value -> DropdownMenuItem({ expanded = false; choose(value) }) {
                        Text(when (value) { EraserChoice.Stroke -> "Cut vector strokes"; EraserChoice.Object -> "Erase whole objects"; EraserChoice.Mask -> "Subtract selected mask" })
                    } }
                }
            }
            if (active || mask) TextButton(drawing, enabled = !busy) { Text("Return to drawing") }
            if (state.busy) TextButton(controller.interaction::cancelOperation) { Text("Cancel erase") }
        }
        if (active || mask) {
            val radius = if (mask) selection.radius else state.radius
            Text("Radius ${radius.toInt()} document px", style = MaterialTheme.typography.caption)
            Slider(radius.toFloat(), { controller.tools.radius(it.toDouble()) }, valueRange = .5f..128f, enabled = !busy)
            Text(when {
                mask && selection.loading -> "Loading the selected mask…"
                mask && selection.selection == null -> "Choose an existing mask in Selection tools. Erasing does not create a mask."
                mask -> "Subtracts mask coverage. Export the mask or cutout with Selection tools."
                state.mode == EraserMode.Stroke -> "Creates editable filled vector outlines, preserving highlighter alpha. Undo restores raw strokes."
                else -> "Removes whole touched objects; locked objects are kept."
            }, style = MaterialTheme.typography.caption)
            Text("The tinted path shows the sweep. Release to save one undo step.", style = MaterialTheme.typography.caption)
        }
        if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        state.message?.let { Text(it, style = MaterialTheme.typography.caption) }
    }
}
