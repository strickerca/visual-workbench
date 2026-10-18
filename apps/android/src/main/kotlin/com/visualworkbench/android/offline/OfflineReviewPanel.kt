package com.visualworkbench.android.offline

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

/** Callback-only leaf. The app owner keeps attachment/edit/lifetime authority. */
@Composable
internal fun OfflineReviewPanel(
    state: OfflineReviewState,
    restoreDocumentId: String?,
    restoreDocumentLabel: String,
    onRefresh: () -> Unit,
    onFirstPage: () -> Unit,
    onNextPage: () -> Unit,
    onSelect: (String) -> Unit,
    onStageSwap: () -> Unit,
    onStageRestore: (String) -> Unit,
    onSubmit: () -> Unit,
    onKeep: () -> Unit,
    onCancelDraft: () -> Unit,
    onCancelOperation: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier.padding(12.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Offline conflicts", style = MaterialTheme.typography.h6)
        Text("Both values and accepted history are retained. A saved review is submitted only when you choose Submit.", style = MaterialTheme.typography.caption)
        Row {
            TextButton(onRefresh, enabled = !state.busy) { Text("Refresh") }
            TextButton(onFirstPage, enabled = !state.busy && state.afterRow != 0L) { Text("First page") }
            TextButton(onNextPage, enabled = !state.busy && state.hasMore) { Text("Next page") }
        }
        if (state.entries.isEmpty() && !state.busy) Text("No conflicts on this page.")
        state.entries.forEach { entry ->
            TextButton({ onSelect(entry.conflictId) }, enabled = !state.busy) {
                val label = if (entry.deleteVsEdit) "Removed annotation" else offlineReviewPropertyLabel(entry.property)
                Text("${if (state.selected?.conflictId == entry.conflictId) "Selected · " else ""}$label · conflict ${entry.row}")
            }
        }
        state.selected?.let { value ->
            Divider()
            Text(if (value.deleteVsEdit) "Removed annotation" else offlineReviewPropertyLabel(value.property), style = MaterialTheme.typography.subtitle1)
            Text("Status: ${value.progress.name}")
            OfflineReviewValueText("Historic kept value", value.kept)
            OfflineReviewValueText("Retained other value", value.other)
            OfflineReviewValueText("Current project value", value.current)
            value.operation?.let { operation ->
                Text(if (value.deleteVsEdit) "Prepared: create a fresh restored copy" else "Prepared: apply the retained value")
                var showOperation by remember(value.transactionId) { mutableStateOf(false) }
                TextButton({ showOperation = !showOperation }) { Text("Prepared operation details") }
                if (showOperation) OfflineReviewValueText("Prepared operation", operation)
            }
            if (value.staleDraft || state.requiresRefresh) Text("This view changed. Refresh; the saved review remains available.", color = MaterialTheme.colors.error)
            val enabled = !state.busy && !state.requiresRefresh
            if (value.progress == OfflineReviewProgress.Unreviewed) {
                Button(onKeep, enabled = enabled) { Text("Keep current") }
                if (value.deleteVsEdit) {
                    Text("Restore creates a fresh, ungrouped copy in its retained layer. Destination: $restoreDocumentLabel")
                    Button({ restoreDocumentId?.let(onStageRestore) }, enabled = enabled && restoreDocumentId != null) { Text("Preview restored copy") }
                } else Button(onStageSwap, enabled = enabled) { Text("Preview swap to other value") }
            }
            if (value.progress == OfflineReviewProgress.Draft) {
                Button(onSubmit, enabled = enabled && !value.staleDraft && value.completePreview) { Text("Submit saved review") }
                TextButton(onCancelDraft, enabled = enabled) { Text("Cancel saved review") }
                if (!value.completePreview) Text("The bounded preview is incomplete. New submission is unavailable; retained values remain saved.")
            }
            if (value.progress == OfflineReviewProgress.Queued) Text("The existing paired link owns delivery. Use ordinary Undo to reverse this edit.")
            if (value.progress == OfflineReviewProgress.Applied) Text("The review edit was accepted. Use ordinary Undo to reverse it.")
        }
        if (state.busy) {
            LinearProgressIndicator(Modifier.fillMaxWidth())
            TextButton(onCancelOperation) { Text("Stop waiting and refresh status") }
        }
        state.message?.let { Text(it, style = MaterialTheme.typography.caption) }
    }
}
@Composable
private fun OfflineReviewValueText(label: String, value: OfflineReviewValue) {
    Text(label, style = MaterialTheme.typography.caption)
    Text(if (!value.available) "Current value unavailable in this view" else value.preview)
    if (value.truncated) Text("Partial preview · ${value.utf8Bytes} UTF-8 bytes retained", color = MaterialTheme.colors.error)
}
