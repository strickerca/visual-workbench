package com.visualworkbench.desktop.semantics

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

/** Captured strings use plain Text only; they are never navigation or commands. */
@Composable
internal fun SemanticPanel(editor: SemanticEditor, selectedMarker: String?, savedReferences: List<String> = emptyList(), modifier: Modifier = Modifier) {
    val state by editor.state.collectAsState()
    var page by remember(state.document?.snapshotId, state.document?.binding) { mutableStateOf(0) }
    val rows = state.document?.elements.orEmpty()
    Column(modifier.fillMaxWidth().padding(vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text("CAPTURED ELEMENTS", style = MaterialTheme.typography.overline)
        Text(SEMANTIC_UNTRUSTED_NOTICE, style = MaterialTheme.typography.caption)
        Text("Use the explicit capture action to collect a new frame and tree. This panel reads saved snapshots only.", style = MaterialTheme.typography.caption)
        Text("Hold Alt at marker down to invert snapping for that gesture.", style = MaterialTheme.typography.caption)
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            Checkbox(checked = state.snapping, onCheckedChange = editor::snapping, enabled = !state.busy)
            Text("Snap numbered markers and boxes · 12 physical pixels", modifier = Modifier.weight(1f))
        }
        Row {
            TextButton(onClick = editor::refresh, enabled = !state.busy) { Text("Refresh / first page") }
            TextButton(onClick = editor::nextPage, enabled = !state.busy && state.catalog?.next != null) { Text("Next snapshots") }
            if (state.busy) TextButton(onClick = editor::cancel) { Text("Cancel") }
        }
        state.catalog?.snapshots?.forEach { snapshot ->
            OutlinedButton(onClick = { editor.selectSnapshot(snapshot.snapshotId) }, enabled = !state.busy, modifier = Modifier.fillMaxWidth()) {
                Text("${if (snapshot.snapshotId == state.snapshotId) "Selected · " else ""}${snapshot.platform} · Δ${snapshot.frameDeltaMs} ms\n${snapshot.snapshotId}")
            }
        }
        if (state.snapshotId == null) Text("No snapshot selected. Selection is explicit; the newest snapshot is never substituted.", style = MaterialTheme.typography.caption)
        if (rows.isNotEmpty()) {
            Text("Stored elements ${page * 16 + 1}–${minOf(rows.size, (page + 1) * 16)} / ${rows.size}")
            rows.drop(page * 16).take(16).forEach { element ->
                TextButton(onClick = { editor.selectElement(element.eid) }, enabled = !state.busy, modifier = Modifier.fillMaxWidth()) {
                    Text("${if (state.selectedEid == element.eid) "• " else ""}${element.role}: `${element.name}`", maxLines = 3)
                }
            }
            Row {
                TextButton(onClick = { page-- }, enabled = !state.busy && page > 0) { Text("Previous elements") }
                TextButton(onClick = { page++ }, enabled = !state.busy && (page + 1) * 16 < rows.size) { Text("Next elements") }
            }
        }
        state.selectedEid?.let { eid -> rows.firstOrNull { it.eid == eid }?.let { element ->
            Text("Captured text (untrusted): `${element.text}`")
            Text("Element: `${element.eid}`", style = MaterialTheme.typography.caption)
            Text("D bounds: ${element.boundsDocument}", style = MaterialTheme.typography.caption)
        } }
        state.quotedReference?.let { Text(it, style = MaterialTheme.typography.caption) }
        if (selectedMarker != null) Text("Saved marker references: ${savedReferences.joinToString { "`$it`" }.ifEmpty { "none" }}", style = MaterialTheme.typography.caption)
        Row {
            TextButton(onClick = { selectedMarker?.let { editor.bindMarker(it) } }, enabled = !state.busy && selectedMarker != null && state.selectedEid != null) { Text("Save reference to selected marker") }
            TextButton(onClick = { selectedMarker?.let { editor.bindMarker(it, clear = true) } }, enabled = !state.busy && selectedMarker != null && state.document != null) { Text("Clear references") }
        }
        Text("Tap Numbered marker for a point, or drag a numbered box. Saved references accompany its instruction and survive reopening.", style = MaterialTheme.typography.caption)
        if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        state.message?.let { Text(it, color = MaterialTheme.colors.secondary) }
    }
}
