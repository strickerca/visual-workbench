package com.visualworkbench.desktop.remote

import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogWindow
import androidx.compose.ui.window.rememberDialogState
import com.visualworkbench.shared.*
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicReference

/** A separate owner action; importing a receipt never enters this flow. */
@Composable internal fun RemoteDrawingCheckPanel(controller:RemoteEditorPalette) {
    if(!controller.drawingCheckAvailable)return
    val palette by controller.state.collectAsState()
    val check by controller.drawingCheckState.collectAsState()
    val scope=rememberCoroutineScope()
    val active=check.phase in setOf(RemoteDrawingCheckPhase.Preparing,
        RemoteDrawingCheckPhase.AwaitingConfirmation,RemoteDrawingCheckPhase.Running)
    val sessionSealed=palette.target?.binding?.let { binding->palette.sealedBinding?.let {
        it.connectionEpoch==binding.connectionEpoch&&it.inputSessionId==binding.inputSessionId
    }}==true
    TextButton({controller.prepareDrawingCheck()},enabled=!palette.closed&&!palette.busy&&!active&&
        !sessionSealed&&!check.cleanupPending&&palette.target?.identity?.editor==RemoteEditorKind.Paint) {
        Text("Check mouse drawing in Paint")
    }
    if(check.phase==RemoteDrawingCheckPhase.AwaitingConfirmation) {
        val runFocus=remember{FocusRequester()}
        val dialog=remember{AtomicReference<java.awt.Window?>()}
        var dismissed by remember{mutableStateOf(false)}
        fun dismiss(){dismissed=true;dialog.getAndSet(null)?.let{it.isVisible=false;it.dispose()}}
        if(!dismissed)DialogWindow(onCloseRequest={dismiss();controller.cancelDrawingCheck()},title="Check mouse drawing in Paint",
            state=rememberDialogState(width=560.dp,height=360.dp)) {
            SideEffect{dialog.set(window)}
            MaterialTheme {Column(Modifier.fillMaxSize().padding(20.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                Text("Use an opaque white canvas with the brush selected and a contrasting color in Paint. Move the pointer outside the Paint canvas, then press Enter to run. The check brings Paint forward, adds one mouse dot, and uses Undo, Redo and Undo. A redo entry can remain. Physical pen support stays Not tested.")
                Row(horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                    Button({
                        // Actual dialog retirement precedes native foreground
                        // validation; no timer pretends that focus is restored.
                        dismiss()
                        controller.confirmDrawingCheck()
                    },modifier=Modifier.focusRequester(runFocus)){Text("Run check")}
                    TextButton({dismiss();controller.cancelDrawingCheck()}){Text("Cancel")}
                }
            }}
            LaunchedEffect(Unit){runFocus.requestFocus()}
        }
    }
    if(active){LinearProgressIndicator();TextButton({controller.cancelDrawingCheck()}){Text("Cancel drawing check")}}
    check.message?.let{Text(it)}
    if(check.cleanupPending)TextButton({scope.launch {try{controller.retryDrawingCheckCleanup()}catch(_:Exception){}}}){
        Text("Retry drawing check cleanup")
    }
}
