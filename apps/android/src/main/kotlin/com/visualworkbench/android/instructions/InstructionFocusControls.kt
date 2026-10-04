package com.visualworkbench.android.instructions

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.InstructionFocusController

/** Session choices are explicit and reset when the authenticated link closes. */
@Composable
internal fun InstructionFocusControls(controller: InstructionFocusController) {
    val state by controller.state.collectAsState()
    Column {
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), horizontalArrangement = Arrangement.SpaceBetween) {
            Text("Share marker selection", Modifier.weight(1f))
            Switch(state.sharing, controller::sharing, enabled = state.available)
        }
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), horizontalArrangement = Arrangement.SpaceBetween) {
            Text("Follow peer instruction", Modifier.weight(1f))
            Switch(state.following, controller::following, enabled = state.available)
        }
        Text(if (state.available) "Following opens the peer's instruction after your draft is saved or discarded. Your canvas view stays fixed."
            else "Connect a compatible peer to share marker selection.", style = MaterialTheme.typography.caption)
        if (state.peerMarkerId != null) Text("Peer marker selected", style = MaterialTheme.typography.caption)
        state.message?.let { Text(it, color = MaterialTheme.colors.error) }
    }
}
