package com.visualworkbench.desktop.instructions

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.awt.SwingPanel
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable
internal fun InstructionsPanel(editor: InstructionEditor, focus: InstructionFocusController, semantic: SemanticEditor, selected: String?, modifier: Modifier = Modifier, onTextFocus: () -> Unit = {}) {
    val state by editor.state.collectAsState()
    val list = rememberLazyListState()
    val otherRows = state.document?.instructions.orEmpty().filter { row -> state.document?.markers?.none { it.instructionId == row.instructionId } == true }
    LaunchedEffect(state.field?.sessionId) {
        if (state.field != null) list.scrollToItem(1 + state.document?.markers.orEmpty().size + otherRows.size)
    }
    LazyColumn(modifier.padding(12.dp), state = list, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
        Text("INSTRUCTIONS", style = MaterialTheme.typography.overline)
        InstructionFocusControls(focus)
        com.visualworkbench.desktop.semantics.SemanticPanel(semantic, selected?.takeIf { id -> state.document?.markers?.any { it.objectId == id } == true },
            state.document?.markers?.firstOrNull { it.objectId == selected }?.elementEids.orEmpty())
        Text("Use Workbench → Foreground capture for a fresh frame and stored semantic tree.", style = MaterialTheme.typography.caption)
        Row {
            TextButton(onClick = editor::focusGlobal, enabled = !state.busy) { Text("Global") }
            TextButton(onClick = { selected?.let(editor::focusObject) }, enabled = selected != null && !state.busy) { Text("Selected object") }
        }
        if (state.document?.needsReconciliation == true) Text("Marker links require reconciliation. Saved numbering is preserved.", color = MaterialTheme.colors.error)
        }
            items(state.document?.markers.orEmpty(), key = { it.objectId }) { marker ->
                val row = state.document?.instructions?.firstOrNull { it.instructionId == marker.instructionId }
                if (state.field?.instructionId != row?.instructionId) {
                    OutlinedTextField(value = row?.text.orEmpty(), onValueChange = {}, readOnly = true,
                        label = { Text("Marker ${marker.number}${if (marker.hidden || !marker.layerVisible) " · hidden" else ""}") },
                        modifier = Modifier.fillMaxWidth().clickable(enabled = !state.busy) { editor.focusObject(marker.objectId) }, maxLines = 3)
                    TextButton(onClick = { editor.focusObject(marker.objectId) }, enabled = !state.busy) { Text("Edit marker ${marker.number}") }
                } else Text("Marker ${marker.number} · editing below")
            }
            items(otherRows, key = { it.instructionId }) { row ->
                TextButton(onClick = { editor.focusInstruction(row.instructionId) }, enabled = !state.busy) {
                    Text(if (row.targetIds.isEmpty()) "Global instruction" else if (row.detached) "Detached instruction" else "Object instruction")
                }
            }
        state.field?.let { value ->
          item(key = "active-${value.instructionId}") {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Role · independent of color", style = MaterialTheme.typography.caption)
            Row { InstructionRole.entries.forEach { role ->
                TextButton(onClick = { editor.role(role) }, enabled = !state.busy, modifier = Modifier.weight(1f)) { Text(if (role == value.role) "• ${role.name}" else role.name, style = MaterialTheme.typography.caption) }
            } }
            key(value.sessionId) {
                val component = remember { WindowsInstructionField() }
                DisposableEffect(component) { onDispose { component.release() } }
                SwingPanel(factory = { component }, update = {
                    it.changed = { text -> editor.update(text, value.sessionId, value.generation) }
                    it.focused = onTextFocus
                    it.apply(value.text, !state.busy, value.sessionId)
                }, modifier = Modifier.fillMaxWidth().height(170.dp))
            }
            Row {
                TextButton(onClick = { editor.method(InstructionEntryMethod.PcKeyboard) }, enabled = !state.busy) { Text("Keyboard") }
                TextButton(onClick = {
                    if (System.getProperty("os.name").startsWith("Windows", ignoreCase = true)) {
                        editor.method(InstructionEntryMethod.Voice)
                        editor.message("Focus the instruction field and press Win+H. Windows controls microphone consent and recognition. If it reports unavailable, enable Windows voice typing or use Keyboard. Review and Save the resulting text.")
                    } else editor.message("Windows voice typing is unavailable on this OS. Use Keyboard.")
                }, enabled = !state.busy) { Text("Voice typing · Win+H") }
            }
            if (value.stale) { Text("The document changed; the complete draft is retained."); TextButton(onClick = editor::reapply, enabled = !state.busy) { Text("Refresh and reapply draft") } }
            Row {
                Button(onClick = editor::save, enabled = value.dirty && !value.stale && !state.busy) { Text("Save") }
                TextButton(onClick = editor::discard, enabled = !state.busy) { Text("Discard") }
                if (state.busy) TextButton(onClick = editor::cancel) { Text("Cancel") }
                val marker = state.document?.markers?.firstOrNull { it.instructionId == value.instructionId }
                TextButton(onClick = { if (marker != null) editor.deleteMarker(marker.objectId) else editor.deleteInstruction(value.instructionId) }, enabled = !state.busy && !value.dirty && value.existing) { Text("Delete") }
            }
            }
          }
        }
        if (state.busy) item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
        state.message?.let { message -> item { Text(message, style = MaterialTheme.typography.caption, color = MaterialTheme.colors.secondary) } }
    }
}
