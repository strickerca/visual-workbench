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
    choose: (DocumentNavigationRequest) -> Unit, cancel: (WorkflowBinding, Long) -> Unit, modifier: Modifier = Modifier) {
    val state by navigator.state.collectAsState()
    val bound = state.binding
    val epoch = state.attachmentEpoch
    OutlinedButton(onClick = { activate(true) }, enabled = state.binding != null,
        modifier = modifier.heightIn(min = 48.dp)) { Text("Documents (${state.documentIds.size})") }
    if (!state.active) return
    AlertDialog(onDismissRequest = { activate(false) }, title = { Text("Documents in this project") }, text = {
        Column(Modifier.fillMaxWidth()) {
            Text(state.title, style = MaterialTheme.typography.subtitle1)
            state.issue?.let { Text(it, style = MaterialTheme.typography.body2) }
            if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 420.dp)) {
                itemsIndexed(state.documentIds, key = { _, id -> id }) { index, id ->
                    val selected = id == state.binding?.documentId
                    TextButton(onClick = { if (bound != null && epoch != null) navigator.request(bound, epoch, id)?.let(choose) }, enabled = state.ready && !selected,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text("${index + 1}. ${if (selected) state.title + " · current" else "Document"}")
                    }
                }
            }
        }
    }, confirmButton = { TextButton(onClick = { activate(false) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Close") } },
        dismissButton = { if (state.busy) TextButton(onClick = { if (bound != null && epoch != null) cancel(bound, epoch) },
            modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel switch") } })
}
