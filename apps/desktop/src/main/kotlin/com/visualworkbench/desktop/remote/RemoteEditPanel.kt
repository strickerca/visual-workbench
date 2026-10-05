package com.visualworkbench.desktop.remote

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*
import kotlinx.coroutines.*

@Composable internal fun RemoteEditPanel(remote:WorkbenchRemoteEdit,onDismiss:()->Unit) {
    val scope=rememberCoroutineScope();val palette=remember(remote){RemoteEditorPalette(remote,scope)}
    val state by remote.state.collectAsState()
    var windows by remember{mutableStateOf<List<RemoteWindowCandidate>>(emptyList())}
    var selected by remember{mutableStateOf<RemoteWindowCandidate?>(null)}
    var message by remember{mutableStateOf<String?>(null)}
    var busy by remember{mutableStateOf(false)}
    val consumer=remember(remote){object:RemoteConsumerOwner{override fun invalidate(){};override suspend fun retire():Boolean{palette.close();return true}}.also{remote.registerConsumer(it)}}
    fun run(block:suspend()->Unit){if(busy)return;busy=true;scope.launch{try{block();message=null}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){message="The operation was refused or retirement is pending. Recheck the target and retry."}finally{busy=false}}}
    LaunchedEffect(remote){try{windows=remote.windows()}catch(_:Exception){message="Window selection is unavailable."}}
    DisposableEffect(consumer){onDispose{remote.seal("owner_pause");retireRemoteEditLater(remote)}}
    AlertDialog(onDismissRequest={},title={Text("Remote Krita / Paint")},text={Column(Modifier.widthIn(min=480.dp,max=760.dp).heightIn(max=680.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)){
        Text("${state.status.name} · existing physical display")
        (state.destinationLabel?:selected?.let{"${it.label} · ${it.executableName}"})?.let{Text("Selected window: $it")}
        Text("Choose an exact window, then grant on this computer. Windows may require you to focus the editor and press Grant again.")
        TextButton(onClick={run{windows=remote.windows()}},enabled=!busy){Text("Refresh windows")}
        for(candidate in windows){TextButton(onClick={selected=candidate;run{remote.select(candidate)}},enabled=!busy){Text("${candidate.label} · ${candidate.executableName}")}}
        Button(onClick={run{remote.select(checkNotNull(selected));remote.grant()}},enabled=!busy&&selected!=null){Text("Grant selected window")}
        TextButton(onClick={run{remote.pause("owner_pause")}},enabled=!busy){Text("Pause control")}
        state.encoderDescription?.let{Text(it)}
        (message?:state.reason)?.let{Text(it)}
        Text("24 Mbps hardware HEVC requested. 30 fps / 80 ms, physical pen fidelity and editor effects still need acceptance.",style=MaterialTheme.typography.caption)
        RemoteEditorPalettePanel(palette)
    }},confirmButton={TextButton(onClick={run{palette.close();remote.close();onDismiss()}},enabled=!busy){Text("Close remote view")}})
}
