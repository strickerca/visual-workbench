package com.visualworkbench.desktop.remote

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable internal fun RemoteEditorPalettePanel(controller:RemoteEditorPalette) {
    val state by controller.state.collectAsState();val target=state.target
    val unavailable=remoteEditorPaletteStateReason(state)
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(6.dp)) {
        Text("Remote editor controls",style=MaterialTheme.typography.subtitle1)
        Text("Destination: ${target?.identity?.editor?.name ?: "no selected editor"}. Workbench history is separate.",style=MaterialTheme.typography.body2)
        target?.let{selected->val identity=selected.identity
            Text("File version ${identity.fileVersion ?: "unknown"}"+(identity.packageVersion?.let{" · package $it"} ?: ""),style=MaterialTheme.typography.caption)
            remoteEditorActions(identity.editor).forEach { action ->
                OutlinedButton({controller.command(action)},enabled=unavailable==null&&!state.busy&&remoteEditorActionEnabled(selected,action),modifier=Modifier.fillMaxWidth()){Text(remoteEditorActionLabel(identity.editor,action))}
            }
            if(unavailable==null)remoteEditorActions(identity.editor).forEach{action->
                remoteEditorActionUnavailableReason(selected,action)?.let{reason->
                    Text("${remoteEditorActionLabel(identity.editor,action)}: $reason",style=MaterialTheme.typography.caption)
                }
            }
            remoteEditorLimits(identity.editor).forEach{Text(remoteEditorLimitText(it),style=MaterialTheme.typography.caption)}
            if(identity.editor==RemoteEditorKind.Paint) {
                OutlinedButton({controller.observeCompatibility()},enabled=!state.closed&&!state.busy,
                    modifier=Modifier.fillMaxWidth()){Text("Read current Paint controls")}
            }
            RemoteCompatibilityImportButton(controller)
            remoteEditorCompatibilitySummary(state).forEach{Text(it,style=MaterialTheme.typography.caption)}
        }
        unavailable?.let{Text(it,style=MaterialTheme.typography.caption)}
        if(state.busy)LinearProgressIndicator(Modifier.fillMaxWidth())
        state.message?.let{Text(it,style=MaterialTheme.typography.caption)}
    }
}
