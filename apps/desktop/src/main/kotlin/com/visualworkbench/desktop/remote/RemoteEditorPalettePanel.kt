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
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(6.dp)) {
        Text("Remote editor controls",style=MaterialTheme.typography.subtitle1)
        Text("Destination: ${target?.identity?.editor?.name ?: "no selected editor"}. Workbench history is separate.",style=MaterialTheme.typography.body2)
        target?.let{selected->val identity=selected.identity
            Text("File version ${identity.fileVersion ?: "unknown"}"+(identity.packageVersion?.let{" · package $it"} ?: ""),style=MaterialTheme.typography.caption)
            remoteEditorActions(identity.editor).forEach { action ->
                OutlinedButton({controller.command(action)},enabled=!state.closed&&!state.busy&&remoteEditorActionEnabled(selected,action),modifier=Modifier.fillMaxWidth()){Text(remoteEditorActionLabel(identity.editor,action))}
            }
            remoteEditorLimits(identity.editor).forEach{Text(remoteEditorLimitText(it),style=MaterialTheme.typography.caption)}
            Text("Compatibility applies to this editor, tool and settings only.",style=MaterialTheme.typography.caption)
            RemoteCompatibilityAspect.entries.forEach{aspect->val record=state.compatibility.firstOrNull{it.aspect==aspect};Text("${aspect.name}: ${record?.result?.name ?: "Not tested"}",style=MaterialTheme.typography.caption)}
        }
        remoteEditorPaletteReason(target)?.let{Text(it,style=MaterialTheme.typography.caption)}
        if(state.busy)LinearProgressIndicator(Modifier.fillMaxWidth())
        state.message?.let{Text(it,style=MaterialTheme.typography.caption)}
    }
}
