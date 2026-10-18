package com.visualworkbench.android.editor

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

/** The same bounded catalog is used for canvas targets and hidden/locked objects.
 * Local text fields are proposals; only an explicit button submits a transaction. */
@Composable
internal fun DocumentControlsPanel(
    controls: DocumentControlsInteraction,
    newId: () -> String,
    openDocument: (String) -> Unit,
    selected: Set<String>,
    activate: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val state by controls.state.collectAsState()
    OutlinedButton(onClick = { activate(true) }, modifier = modifier.heightIn(min = 48.dp)) {
        Text(if (state.busy) "Documents and layers · saving…" else if (state.retry != null)
            "Documents and layers · retry retained" else "Documents and layers")
    }
    if (!state.active) return
    var name by remember { mutableStateOf("") }
    var percent by remember { mutableStateOf("100") }
    var inspected by remember { mutableStateOf<String?>(null) }
    val snapshot = state.snapshot
    val layer = snapshot?.layers?.firstOrNull { it.layerId == inspected } ?: snapshot?.layers?.firstOrNull()
    val targets = controls.targets()
    val opacity = percent.toDoubleOrNull()?.takeIf { it.isFinite() && it in 0.0..100.0 }?.div(100.0)
    val nameValid = name.isNotBlank() && name.length <= 4096 && name.none { it.isISOControl() }
    fun send(command: DocumentControlCommand) { controls.execute(command) }
    AlertDialog(onDismissRequest = { activate(false) },
        title = { Text("Documents and layers") },
        text = {
            LazyColumn(Modifier.fillMaxWidth().heightIn(min = 200.dp, max = 560.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)) {
                item {
                    if (state.busy || state.loading) LinearProgressIndicator(Modifier.fillMaxWidth())
                    state.issue?.let { Text(it, style = MaterialTheme.typography.body2) }
                    state.lastReceipt?.let { Text("Saved at revision ${it.revision.hostSeq}. Undo remains available in the editor.",
                        style = MaterialTheme.typography.caption) }
                    if (state.retry != null) {
                        Text("The exact edit is retained. Retry learns whether it was saved; discarding the handle never undoes a saved edit.")
                        Row {
                            TextButton(onClick = { controls.retry() }, enabled = !state.busy) { Text("Retry exact edit") }
                            TextButton(onClick = { controls.discardRetry() }, enabled = !state.busy) { Text("Discard retry") }
                        }
                    }
                    if (state.busy) TextButton(onClick = controls::cancel) { Text("Cancel and settle") }
                    TextButton(onClick = controls::reload, enabled = !state.busy) { Text("Reload controls") }
                    OutlinedTextField(name, { if (it.length <= 4096) name = it }, label = { Text("Name for next action") },
                        enabled = !state.busy, modifier = Modifier.fillMaxWidth())
                }
                item {
                    Text("Documents", style = MaterialTheme.typography.subtitle1)
                    Text("A new document keeps this immutable original and starts with empty annotations.",
                        style = MaterialTheme.typography.caption)
                    Row {
                        TextButton(onClick = { send(DocumentControlCommand.RenameDocument(name)) },
                            enabled = state.ready && nameValid) { Text("Rename current") }
                        TextButton(onClick = { send(DocumentControlCommand.CreateDocumentFromSource(newId(),newId(),name)) },
                            enabled = state.ready && nameValid && snapshot?.documents?.size?.let { it < 128 } == true) { Text("New document") }
                    }
                }
                items(snapshot?.documents.orEmpty(), key = { "doc:${it.documentId}" }) { row ->
                    TextButton(onClick = { openDocument(row.documentId) }, enabled = state.ready && row.documentId != snapshot?.binding?.documentId,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text((if (row.documentId == snapshot?.binding?.documentId) "Current · " else "") +
                            "${row.title} · ${row.width} × ${row.height}")
                    }
                }
                item {
                    Divider()
                    Text("Layers · back to front", style = MaterialTheme.typography.subtitle1)
                    TextButton(onClick = { send(DocumentControlCommand.CreateAnnotationLayer(newId(),name)) },
                        enabled = state.ready && nameValid && snapshot?.layers?.size?.let { it < 256 } == true) { Text("Add annotation layer") }
                }
                items(snapshot?.layers.orEmpty(), key = { "layer:${it.layerId}" }) { row ->
                    TextButton(onClick = { inspected = row.layerId }, enabled = state.ready,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text((if (row.layerId == layer?.layerId) "Selected · " else "") +
                            "${row.name} · ${row.kind} · ${row.objectCount} objects" +
                            (if (!row.visible) " · hidden" else "") + (if (row.locked) " · locked" else "") +
                            (if (state.drawingLayerId == row.layerId) " · drawing layer" else ""))
                    }
                }
                item {
                    layer?.let { current ->
                        val order = snapshot?.layers.orEmpty()
                        val index = order.indexOfFirst { it.layerId == current.layerId }
                        val editable = state.ready && !current.locked
                        TextButton(onClick = { controls.drawingLayer(current.layerId) },
                            enabled = editable && current.visible && current.kind == "annotation") { Text("Draw in this layer") }
                        Row {
                            TextButton(onClick = { send(DocumentControlCommand.UpdateLayer(LayerControlUpdate(current.layerId,visible=!current.visible))) },
                                enabled = editable) { Text(if (current.visible) "Hide layer" else "Show layer") }
                            TextButton(onClick = { send(DocumentControlCommand.UpdateLayer(LayerControlUpdate(current.layerId,locked=!current.locked))) },
                                enabled = state.ready) { Text(if (current.locked) "Unlock layer" else "Lock layer") }
                        }
                        Row {
                            TextButton(onClick = { send(DocumentControlCommand.PlaceLayer(current.layerId,order.getOrNull(index+2)?.layerId)) },
                                enabled = editable && index >= 0 && index < order.lastIndex) { Text("Raise") }
                            TextButton(onClick = { send(DocumentControlCommand.PlaceLayer(current.layerId,order.getOrNull(index-1)?.layerId)) },
                                enabled = editable && index > 0) { Text("Lower") }
                            TextButton(onClick = { send(DocumentControlCommand.UpdateLayer(LayerControlUpdate(current.layerId,name=name))) },
                                enabled = editable && nameValid) { Text("Rename layer") }
                        }
                        OutlinedTextField(percent, { if (it.length <= 16) percent=it }, label = { Text("Layer opacity · 0–100%") },
                            enabled = editable, modifier = Modifier.fillMaxWidth())
                        Row {
                            TextButton(onClick = { opacity?.let { send(DocumentControlCommand.UpdateLayer(LayerControlUpdate(current.layerId,opacity=it))) } },
                                enabled = editable && opacity != null) { Text("Apply opacity") }
                            TextButton(onClick = { send(DocumentControlCommand.UpdateLayer(LayerControlUpdate(current.layerId,blend=if(current.blend=="normal")"multiply" else "normal"))) },
                                enabled = editable && current.kind == "annotation") { Text("Blend: ${current.blend}") }
                        }
                        TextButton(onClick = { send(DocumentControlCommand.DeleteEmptyLayer(current.layerId)) },
                            enabled = editable && current.objectCount == 0u) { Text("Delete empty layer") }
                    }
                }
                item {
                    Divider()
                    Text("Objects and groups", style = MaterialTheme.typography.subtitle1)
                    Text("Hidden and locked objects remain listed. Group actions include nested members and refuse locked changes.",
                        style = MaterialTheme.typography.caption)
                    Row {
                        TextButton(onClick = { controls.selectObjects(selected) }, enabled = state.ready && selected.isNotEmpty()) { Text("Use canvas selection") }
                        TextButton(onClick = { controls.selectObjects(emptySet()) }, enabled = state.ready) { Text("Clear targets") }
                    }
                    Row {
                        TextButton(onClick = { send(DocumentControlCommand.Group(newId(),state.selectedObjects.toList().sorted())) },
                            enabled = state.ready && state.selectedObjects.size in 2..256) { Text("Group targets") }
                        TextButton(onClick = { state.selectedGroup?.let { send(DocumentControlCommand.Ungroup(it)) } },
                            enabled = state.ready && state.selectedGroup != null) { Text("Ungroup") }
                    }
                    Row {
                        TextButton(onClick = { targets?.let { send(DocumentControlCommand.SetObjectLocks(it,true)) } }, enabled = state.ready && targets != null) { Text("Lock targets") }
                        TextButton(onClick = { targets?.let { send(DocumentControlCommand.SetObjectLocks(it,false)) } }, enabled = state.ready && targets != null) { Text("Unlock targets") }
                    }
                    Row {
                        TextButton(onClick = { targets?.let { send(DocumentControlCommand.SetObjectHidden(it,true)) } }, enabled = state.ready && targets != null) { Text("Hide targets") }
                        TextButton(onClick = { targets?.let { send(DocumentControlCommand.SetObjectHidden(it,false)) } }, enabled = state.ready && targets != null) { Text("Show targets") }
                    }
                    TextButton(onClick = { val to=layer; if (to!=null && targets!=null) send(DocumentControlCommand.MoveObjects(targets,to.layerId)) },
                        enabled = state.ready && targets != null && layer?.kind=="annotation" && layer?.locked==false) { Text("Move targets to selected layer") }
                }
                items(snapshot?.groups.orEmpty(), key = { "group:${it.groupId}" }) { row ->
                    val number = snapshot?.groups?.indexOf(row)?.plus(1)
                    TextButton(onClick = { controls.selectGroup(row.groupId) }, enabled = state.ready,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text((if (state.selectedGroup==row.groupId) "Selected · " else "") +
                            "Group $number" + (if(row.parentId!=null)" · nested" else ""))
                    }
                }
                items(snapshot?.objects.orEmpty(), key = { "object:${it.objectId}" }) { row ->
                    Row(Modifier.fillMaxWidth()) {
                        Checkbox(row.objectId in state.selectedObjects, onCheckedChange = { checked ->
                            controls.selectObjects(if(checked)state.selectedObjects+row.objectId else state.selectedObjects-row.objectId)
                        }, enabled = state.ready && (row.objectId in state.selectedObjects || state.selectedObjects.size < 256))
                        Column(Modifier.weight(1f).padding(vertical=10.dp)) {
                            Text(row.kind + (if(row.hidden)" · hidden" else "") + (if(row.locked)" · locked" else ""))
                            Text(snapshot?.layers?.firstOrNull { it.layerId==row.layerId }?.name.orEmpty() +
                                (if(row.groupId!=null)" · grouped" else ""), style=MaterialTheme.typography.caption)
                        }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { activate(false) }, modifier = Modifier.heightIn(min=48.dp)) { Text("Close panel") } })
}

