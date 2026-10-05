package com.visualworkbench.android.remote

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import android.content.Context
import android.content.ContextWrapper
import androidx.lifecycle.LifecycleOwner
import androidx.compose.ui.platform.LocalContext
import com.visualworkbench.shared.WorkbenchRemoteEdit
import com.visualworkbench.shared.retireRemoteEditLater
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalCoroutinesApi::class)
@Composable internal fun RemoteEditPanel(remote:WorkbenchRemoteEdit,onDismiss:()->Unit) {
    val ownerScope=rememberCoroutineScope()
    val controller=remember(remote){RemoteEditController(remote,ownerScope)}
    val state by remote.state.collectAsState();val display by controller.display.collectAsState();val echo by controller.echo.collectAsState()
    val context=LocalContext.current
    val lifecycle=remember(context){remoteLifecycleOwner(context)}
    val inputAdmission=remember(remote){RemoteUiInputAdmission()}
    var diagnostics by remember{mutableStateOf(false)}
    var closing by remember{mutableStateOf(false)}
    var closeMessage by remember{mutableStateOf<String?>(null)}
    var commandBusy by remember{mutableStateOf(false)}
    var commandMessage by remember{mutableStateOf<String?>(null)}
    fun command(action:RemoteUiCommand) {
        if(closing||commandBusy||!remoteUiCommandEnabled(action,remote.state.value))return
        val captured=if(action==RemoteUiCommand.RequestControl)inputAdmission.capture(remote.state.value.scope)else null
        commandBusy=true;commandMessage=null
        if(action==RemoteUiCommand.Pause){inputAdmission.revoke();controller.deactivate("owner_pause")}
        ownerScope.launch(start=CoroutineStart.ATOMIC) {
            try {
                ensureActive()
                commandMessage=runRemoteUiCommandWithAdmission(action,inputAdmission,captured,
                    {remote.state.value},{closing},{lifecycle.lifecycle.currentState==Lifecycle.State.RESUMED},
                    {when(action){RemoteUiCommand.RequestControl->remote.requestControl();RemoteUiCommand.Pause->remote.pause("owner_pause")}},
                    controller::resumeInput)
            }finally{commandBusy=false}
        }
    }
    fun close(){if(closing)return;closing=true;inputAdmission.revoke();controller.invalidate();ownerScope.launch{try{withContext(NonCancellable){remote.close()};onDismiss()}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){closeMessage="Retirement is pending. Keep this view open and retry Close."}finally{closing=false}}}
    DisposableEffect(controller,lifecycle){
        val observer=LifecycleEventObserver{_,event->when(event){
            Lifecycle.Event.ON_PAUSE,Lifecycle.Event.ON_STOP->{inputAdmission.revoke();controller.deactivate("background");ownerScope.launch{controller.background()}}
            Lifecycle.Event.ON_RESUME->{inputAdmission.revoke();controller.foreground()}
            else->Unit
        }}
        lifecycle.lifecycle.addObserver(observer)
        onDispose{inputAdmission.revoke();lifecycle.lifecycle.removeObserver(observer);controller.deactivate("background");retireRemoteEditLater(remote)}
    }
    Dialog(onDismissRequest={close()},properties=DialogProperties(usePlatformDefaultWidth=false,dismissOnBackPress=false,dismissOnClickOutside=false)){
        Surface(Modifier.fillMaxSize()){Column{
            Text("Remote computer · ${state.status.name}",Modifier.padding(12.dp))
            state.destinationLabel?.let{Text("Selected window: $it",Modifier.padding(horizontal=12.dp))}
            Row(Modifier.padding(horizontal=12.dp),horizontalArrangement=Arrangement.spacedBy(8.dp)){
                Button(onClick={command(RemoteUiCommand.RequestControl)},enabled=!closing&&!commandBusy&&remoteUiCommandEnabled(RemoteUiCommand.RequestControl,state)){Text("Request PC control")}
                TextButton(onClick={command(RemoteUiCommand.Pause)},enabled=!closing&&!commandBusy&&remoteUiCommandEnabled(RemoteUiCommand.Pause,state)){Text("Pause")}
                TextButton(onClick={close()},enabled=!closing){Text("Close")}
            }
            Box(Modifier.weight(1f).fillMaxWidth()){
                AndroidView(factory={context->RemoteSurfaceView(context,controller)},update={it.configure(display.config)},modifier=Modifier.fillMaxSize())
                RemoteGhostOverlay(controller.ghost,state.target?.binding,state.target?.grantActive==true,Modifier.fillMaxSize())
            }
            (closeMessage?:commandMessage?:display.message?:state.reason)?.let{Text(it,Modifier.padding(8.dp))}
            Text("Video continues during pen contact. Hardware performance and physical pen fidelity remain unverified.",Modifier.padding(8.dp),style=MaterialTheme.typography.caption)
            TextButton(onClick={diagnostics=!diagnostics}){Text(if(diagnostics)"Hide diagnostics" else "Show diagnostics")}
            if(diagnostics){
                Text("Pen inversion is unavailable from this Android route. ${controller.inputClockDescription}.",Modifier.padding(8.dp),style=MaterialTheme.typography.caption)
                Text("Validated render callbacks: ${echo.observed}; missing: ${echo.missing}; pending: ${echo.pending}. Callback p95: ${echo.p95Nanos?.let{it/1_000_000L}?:"—"} ms. Editor completion and photons are unmeasured.",Modifier.padding(8.dp),style=MaterialTheme.typography.caption)
            }
            RemoteEditorPalettePanel(controller.palette)
        }}
    }
}

private fun remoteLifecycleOwner(context:Context):LifecycleOwner {
    var current=context
    while(current is ContextWrapper){
        if(current is LifecycleOwner)return current
        val base=current.baseContext
        if(base===current)break
        current=base
    }
    return current as? LifecycleOwner ?: error("Remote view requires an activity lifecycle owner")
}
