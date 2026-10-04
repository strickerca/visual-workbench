package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable
internal fun EraserPanel(controller: EraserController, choose: (EraserChoice) -> Unit,
    drawing: () -> Unit, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    val mask by controller.tools.selection.state.collectAsState()
    val maskActive = mask.active && mask.tool == SelectionTool.MaskEraser
    val active = state.active && !mask.active
    val busy = state.busy || mask.busy
    Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("ERASER", style = MaterialTheme.typography.overline)
        for (choice in EraserChoice.entries) {
            val chosen = when (choice) { EraserChoice.Mask -> maskActive; EraserChoice.Stroke -> active && state.mode == EraserMode.Stroke; EraserChoice.Object -> active && state.mode == EraserMode.Object }
            OutlinedButton({ choose(choice) }, Modifier.fillMaxWidth(), enabled = !busy) {
                Text((if (chosen) "• " else "") + when (choice) { EraserChoice.Stroke -> "Cut strokes"; EraserChoice.Object -> "Erase objects"; EraserChoice.Mask -> "Subtract mask" })
            }
        }
        if (active || maskActive) {
            val radius = if (maskActive) mask.radius else state.radius
            Text("Radius ${radius.toInt()} document px", style = MaterialTheme.typography.caption)
            Slider(radius.toFloat(), { controller.tools.radius(it.toDouble()) }, valueRange = .5f..128f, enabled = !busy)
            Text(when {
                maskActive && mask.loading -> "Loading the selected mask…"
                maskActive && mask.selection == null -> "Select an existing mask with Selection tools. Erasing cannot create a new mask."
                maskActive -> "Subtracts coverage from the selected mask. Use Selection tools to export its PNG."
                state.mode == EraserMode.Stroke -> "Cuts raw strokes and fill-only outlines. Pieces keep one fill and highlighter alpha; Undo restores the original strokes."
                else -> "Deletes whole touched objects. Locked objects are kept."
            }, style = MaterialTheme.typography.caption)
            Text("The tinted path previews the sweep. Release to save one undo step.", style = MaterialTheme.typography.caption)
            TextButton(drawing, enabled = !busy) { Text("Return to drawing") }
        }
        if (state.busy) { LinearProgressIndicator(Modifier.fillMaxWidth()); TextButton(controller.interaction::cancelOperation) { Text("Cancel erase") } }
        state.message?.let { Text(it, style = MaterialTheme.typography.caption) }
    }
}
