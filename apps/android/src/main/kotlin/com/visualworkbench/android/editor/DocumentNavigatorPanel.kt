package com.visualworkbench.android.editor

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

/** The callback runs the editor's normal guarded document switch. Closing this
 * dialog hides it; Cancel asks the root to stop and join the current switch. */
@Composable
internal fun DocumentNavigatorPanel(navigator: DocumentNavigator, activate: (Boolean) -> Unit,
    choose: (DocumentNavigationRequest) -> Unit, cancel: (WorkflowBinding, Long) -> Unit,
    modifier: Modifier = Modifier) {
    DocumentNavigatorPanel(navigator, activate, choose, cancel, null, null, modifier)
}

@Composable
internal fun DocumentNavigatorPanel(navigator: DocumentNavigator, activate: (Boolean) -> Unit,
    choose: (DocumentNavigationRequest) -> Unit, cancel: (WorkflowBinding, Long) -> Unit,
    controls: DocumentControlsInteraction?, newId: (() -> String)?, modifier: Modifier = Modifier) {
    val state by navigator.state.collectAsState()
    val actions = controls?.state?.collectAsState()?.value ?: DocumentControlsState()
    val bound = state.binding
    val epoch = state.attachmentEpoch
    OutlinedButton(onClick = { activate(true) }, enabled = state.binding != null,
        modifier = modifier.heightIn(min = 48.dp)) { Text("Documents (${state.documentIds.size})") }
    if (!state.active) return
    val token = controls?.capture()?.takeIf { it.binding == bound && it.attachmentEpoch == epoch }
    val catalog = documentActionCatalog(state, actions)
    val current = bound?.documentId?.let(catalog::get)
    var title by remember(token) { mutableStateOf(current?.title ?: state.title) }
    val form = DocumentActionForm(title)
    val rename = current?.let(form::rename)
    val createReady = current != null && newId != null && form.valid && current.kind in listOf("image", "capture") &&
        state.documentIds.size in 1..127
    val actionReady = state.ready && token != null && actions.ready && !state.busy
    AlertDialog(onDismissRequest = { activate(false) }, title = { Text("Documents in this project") }, text = {
        Column(Modifier.fillMaxWidth()) {
            Text(state.title, style = MaterialTheme.typography.subtitle1)
            state.issue?.let { Text(it, style = MaterialTheme.typography.body2) }
            if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (actions.loading || actions.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            actions.issue?.let { Text(it, style = MaterialTheme.typography.body2) }
            actions.lastReceipt?.let { Text("Saved at revision ${it.revision.hostSeq}. Undo remains available in the editor.",
                style = MaterialTheme.typography.caption) }
            if (actions.retry != null) {
                Text("The exact document edit is retained. Discarding its retry handle does not undo a saved edit.")
                Row {
                    TextButton(onClick = { token?.let { controls?.retry(it) } }, enabled = !actions.busy) { Text("Retry exact edit") }
                    TextButton(onClick = { token?.let { controls?.discardRetry(it) } }, enabled = !actions.busy) { Text("Discard retry") }
                }
            }
            if (actions.busy) TextButton(onClick = { token?.let { controls?.cancel(it) } }) { Text("Cancel and settle") }
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 420.dp)) {
                if (actions.active) item {
                    Divider()
                    OutlinedTextField(title, { if (it.length <= 4096) title = it },
                        label = { Text("Document title") }, enabled = actionReady,
                        modifier = Modifier.fillMaxWidth())
                    Column {
                        TextButton(onClick = { controls?.let { owner ->
                            if (token != null && owner.capture() == token && rename != null) owner.execute(token, rename)
                        } },
                            enabled = actionReady && rename != null) { Text("Rename current") }
                        TextButton(onClick = {
                            controls?.let { owner ->
                                if (token != null && owner.capture() == token && current != null && newId != null)
                                    form.createFromSource(current, state.documentIds.size, newId)?.let { owner.execute(token, it) }
                            }
                        }, enabled = actionReady && createReady) { Text("New from original") }
                    }
                    Text("New from original starts with an empty annotation layer; it does not copy this document's marks.",
                        style = MaterialTheme.typography.caption)
                }
                itemsIndexed(state.documentIds, key = { _, id -> id }) { index, id ->
                    val selected = id == state.binding?.documentId
                    TextButton(onClick = { if (bound != null && epoch != null) navigator.request(bound, epoch, id)?.let(choose) }, enabled = state.ready && !selected,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text("${index + 1}. ${catalog[id]?.title ?: if (selected) state.title else "Document"}${if (selected) " · current" else ""}")
                    }
                }
            }
        }
    }, confirmButton = { TextButton(onClick = { activate(false) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Close") } },
        dismissButton = { if (state.busy) TextButton(onClick = { if (bound != null && epoch != null) cancel(bound, epoch) },
            modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel switch") } })
}
