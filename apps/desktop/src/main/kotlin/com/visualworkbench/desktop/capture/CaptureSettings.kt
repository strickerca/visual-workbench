package com.visualworkbench.desktop.capture

import androidx.compose.foundation.layout.Column
import androidx.compose.material.*
import androidx.compose.runtime.*

@Composable internal fun CaptureSettings(controller:DesktopCaptureController,onClose:()->Unit){
    var key by remember{mutableStateOf("A")};val enabled by controller.hotkeyEnabled.collectAsState()
    AlertDialog(onDismissRequest=onClose,title={Text("Foreground capture")},text={Column{
        Text("Capture reads only the foreground application after you press the enabled hotkey. Screenshots and bounded accessibility information are saved locally. Visual Workbench cannot be selected as the target. The Windows border indicator remains enabled.")
        OutlinedTextField(key,{key=it.uppercase().take(1)},label={Text("Ctrl + Alt + key")},enabled=!enabled)
        Text("HDR/wide-gamut or unsupported pixel formats are refused; no silent color conversion is used.")
    }},confirmButton={Button(onClick={if(enabled){controller.disableHotkey()}else if(key.singleOrNull()?.let{it in 'A'..'Z'||it in '0'..'9'}==true){controller.enableHotkey(3u,key.single().code.toUInt())}}){Text(if(enabled)"Disable hotkey" else "Enable hotkey")}},dismissButton={TextButton(onClick=onClose){Text("Close")}})
}
