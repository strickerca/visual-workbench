package com.visualworkbench.android.remote

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable internal fun RemoteEditorPalettePanel(controller:RemoteEditorPalette) {
    val state by controller.state.collectAsState();val target=state.target
    Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(4.dp)) {
        Text("Remote destination: ${target?.identity?.editor?.name ?: "none"}",style=MaterialTheme.typography.subtitle2)
        Text("These commands use the PC editor's history and tools. Workbench undo stays separate.",style=MaterialTheme.typography.caption)
        target?.let{selected->val identity=selected.identity
            Text("File ${identity.fileVersion ?: "unknown"}"+(identity.packageVersion?.let{" · package $it"} ?: ""),style=MaterialTheme.typography.caption)
            Row(Modifier.horizontalScroll(rememberScrollState())) {
                remoteEditorActions(identity.editor).forEach { action ->
                    TextButton({controller.command(action)},enabled=!state.closed&&!state.busy&&remoteEditorActionEnabled(selected,action)){Text(remoteEditorActionLabel(identity.editor,action))}
                }
            }
            remoteEditorLimits(identity.editor).forEach{Text(remoteEditorLimitText(it),style=MaterialTheme.typography.caption)}
            val observed=state.compatibility
            Text(if(observed.isEmpty())"Tool pressure, hover, tilt, eraser and buttons: not tested." else observed.joinToString(" · "){"${it.aspect.name}: ${it.result.name}"},style=MaterialTheme.typography.caption)
        }
        remoteEditorPaletteReason(target)?.let{Text(it,style=MaterialTheme.typography.caption)}
        if(state.busy)LinearProgressIndicator(Modifier.fillMaxWidth())
        state.message?.let{Text(it,style=MaterialTheme.typography.caption)}
    }
}
