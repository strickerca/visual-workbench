package com.visualworkbench.desktop.remote

import androidx.compose.material.*
import androidx.compose.runtime.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.launch

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
        AlertDialog(onDismissRequest={controller.cancelDrawingCheck()},
            title={Text("Check mouse drawing in Paint")},
            text={Text("Use a blank visible canvas with the brush selected in Paint. The check moves the pointer, adds one mouse dot, then uses Undo, Redo and Undo. A redo entry can remain. Physical pen support stays Not tested.")},
            confirmButton={TextButton({controller.confirmDrawingCheck()}){Text("Run check")}},
            dismissButton={TextButton({controller.cancelDrawingCheck()}){Text("Cancel")}})
    }
    if(active){LinearProgressIndicator();TextButton({controller.cancelDrawingCheck()}){Text("Cancel drawing check")}}
    check.message?.let{Text(it)}
    if(check.cleanupPending)TextButton({scope.launch {try{controller.retryDrawingCheckCleanup()}catch(_:Exception){}}}){
        Text("Retry drawing check cleanup")
    }
}
