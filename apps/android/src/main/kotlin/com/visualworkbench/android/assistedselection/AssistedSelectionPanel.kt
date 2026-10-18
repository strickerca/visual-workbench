package com.visualworkbench.android.assistedselection

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

/** The editor maps real pen taps with its existing captured-camera path and calls
 * appendAssistedPenPrompt, then actions.prompt. This leaf owns no canvas,
 * project, model, mutation dispatcher or manual selection controller. */
@Composable
internal fun AssistedSelectionPanel(actions: AssistedSelectionInteraction,
    prompt: AssistedSelectionPrompt, negative: Boolean, onNegative: (Boolean) -> Unit,
    onClearPrompts: () -> Unit, target: SelectionTarget?, onManualSelection: (String?) -> Unit,
    onDismiss: () -> Unit) {
    val state by actions.state.collectAsState()
    var combine by remember(actions) { mutableStateOf(SelectionCombine.Add) }
    val proposal = state.proposal
    AlertDialog(onDismissRequest = onDismiss, title = { Text("Assisted selection") }, text = {
        Column(Modifier.heightIn(max = 600.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Use pen taps to include or exclude pixels, review the proposed coverage, then Accept to save one selection edit.")
            state.issue?.let { Text(it) }
            if (state.canPrompt) {
                Row {
                    TextButton({ onNegative(false) }, enabled = state.canPrompt) { Text(if (!negative) "• Include" else "Include") }
                    TextButton({ onNegative(true) }, enabled = state.canPrompt) { Text(if (negative) "• Exclude" else "Exclude") }
                }
                Text("${prompt.positive.size} include · ${prompt.negative.size} exclude · at most 64 real taps")
                TextButton(onClearPrompts) { Text("Clear pen prompts") }
                OutlinedButton({ actions.prompt(prompt) }, enabled = prompt.positive.isNotEmpty() || prompt.box != null) {
                    Text("Propose from current prompts")
                }
            }
            if (state.phase in setOf(AssistedSelectionPhase.Encoding, AssistedSelectionPhase.Proposing, AssistedSelectionPhase.Saving))
                LinearProgressIndicator(Modifier.fillMaxWidth())
            state.execution?.let { Text("Local execution: ${it.name}", style = MaterialTheme.typography.caption) }
            if (proposal != null) {
                AssistedCoveragePreview(proposal)
                if (target is SelectionTarget.Existing) Row {
                    SelectionCombine.entries.forEach { value ->
                        TextButton({ combine = value }, enabled = state.canAccept) {
                            Text((if (combine == value) "• " else "") + value.name)
                        }
                    }
                }
                OutlinedButton({ if (target != null) actions.accept(proposal, target,
                    if (target is SelectionTarget.New) SelectionCombine.Add else combine) },
                    enabled = state.canAccept && target != null) { Text("Accept proposal") }
                Text("Saved masks use the existing Add/Subtract brushes, invert, expand, shrink and feather controls.",
                    style = MaterialTheme.typography.caption)
            }
            val saved = state.receipt
            if (saved != null) TextButton({ onManualSelection(saved.snapshot.selection.objectId) }) { Text("Correct saved mask") }
            else TextButton({ onManualSelection(null) }) { Text("Use manual selection") }
            if (state.canPrompt || state.phase == AssistedSelectionPhase.Saving)
                TextButton(actions::cancel) { Text(if (state.phase == AssistedSelectionPhase.Saving) "Cancel and settle import" else "Cancel proposal") }
        }
    }, confirmButton = { TextButton(onDismiss) { Text("Done") } })
}

@Composable
private fun AssistedCoveragePreview(proposal: AssistedSelectionProposal) {
    val region = remember(proposal) {
        val width = minOf(64u, proposal.source.document.width)
        val height = minOf(64u, proposal.source.document.height)
        SelectionRegion((proposal.source.document.width - width) / 2u,
            (proposal.source.document.height - height) / 2u, width, height)
    }
    val tile = remember(proposal) { proposal.tile(region) }
    Text("Mask coverage · ${region.width} × ${region.height} at (${region.x}, ${region.y})" +
        if (region.width != proposal.source.document.width || region.height != proposal.source.document.height) " · centre crop" else "")
    // At most 4096 draw commands and 4 KiB of preview coverage, inside the
    // owner's prompt/preview reserve. No full bitmap or native texture is made.
    Canvas(Modifier.size(192.dp).background(Color.Black)) {
        val cellWidth = size.width / region.width.toFloat()
        val cellHeight = size.height / region.height.toFloat()
        tile.coverage.forEachIndexed { index, byte ->
            val alpha = byte.toUByte().toInt() / 255f
            if (alpha > 0f) drawRect(Color(1f, 0f, 1f, alpha),
                Offset((index % region.width.toInt()) * cellWidth, (index / region.width.toInt()) * cellHeight),
                Size(cellWidth, cellHeight))
        }
    }
}

/** Input adapter only. The caller supplies a real already-mapped document tap;
 * stale source admission is repeated by AssistedSelectionInteraction.prompt. */
internal fun appendAssistedPenPrompt(previous: AssistedSelectionPrompt, point: Point,
    negative: Boolean, source: AssistedSelectionSource): AssistedSelectionPrompt? {
    if (previous.positive.size > 64 || previous.negative.size > 64 ||
        previous.positive.size + previous.negative.size >= 64 ||
        !point.x.isFinite() || !point.y.isFinite() || point.x < 0.0 || point.y < 0.0 ||
        point.x >= source.document.width.toDouble() || point.y >= source.document.height.toDouble()) return null
    return if (negative) previous.copy(negative = previous.negative + point)
        else previous.copy(positive = previous.positive + point)
}
