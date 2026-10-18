package com.visualworkbench.android.documents

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.android.instructions.InstructionFocusModal
import com.visualworkbench.shared.*

/** Picker tickets deliberately die with this composition and project lifetime.
 * A returned URI is never rebound after a project replacement or process death. */
@Composable internal fun DocumentOpenButton(editor:EditorController) {
    var kind by remember{mutableStateOf(EditableKind.Pdf)}
    var launched by remember{mutableStateOf(false)}
    var epoch by remember{mutableStateOf<Long?>(null)}
    var source by remember{mutableStateOf<Uri?>(null)}
    var password by remember{mutableStateOf("")}
    val picker=rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()){uri->
        launched=false
        if(uri!=null && epoch==editor.documentEpoch()) {
            if(kind==EditableKind.Pdf)source=uri else editor.importDocument(uri,kind)
        }
        if(source==null){epoch=null;editor.instructionModal(InstructionFocusModal.DocumentPicker,false)}
    }
    DisposableEffect(editor){onDispose{editor.instructionModal(InstructionFocusModal.DocumentPicker,false)}}
    Row(Modifier.fillMaxWidth().padding(horizontal=8.dp)) {
        listOf(EditableKind.Pdf,EditableKind.Svg).forEach{choice->TextButton({
            if(!launched && source==null){kind=choice;epoch=editor.documentEpoch();launched=true;editor.instructionModal(InstructionFocusModal.DocumentPicker,true);try{picker.launch(arrayOf(if(choice==EditableKind.Pdf)"application/pdf" else "image/svg+xml"))}catch(_:Exception){launched=false;epoch=null;editor.instructionModal(InstructionFocusModal.DocumentPicker,false)}}
        },enabled=!editor.busy&&editor.pending==0&&!launched&&source==null){Text("Open ${choice.name}")}}
    }
    source?.let{uri->AlertDialog(onDismissRequest={source=null;password="";epoch=null;editor.instructionModal(InstructionFocusModal.DocumentPicker,false)},title={Text("PDF password, if needed")},text={OutlinedTextField(password,{password=it.take(1024)},visualTransformation=androidx.compose.ui.text.input.PasswordVisualTransformation(),label={Text("Optional password")})},confirmButton={TextButton({val saved=password.takeIf(String::isNotEmpty);password="";source=null;editor.instructionModal(InstructionFocusModal.DocumentPicker,false);if(epoch==editor.documentEpoch())editor.importDocument(uri,EditableKind.Pdf,saved);epoch=null}){Text("Open PDF")}},dismissButton={TextButton({source=null;password="";epoch=null;editor.instructionModal(InstructionFocusModal.DocumentPicker,false)}){Text("Cancel")}})}
}

@Composable internal fun DocumentControls(editor:EditorController) {
    var ticket by remember{mutableStateOf<DocumentExport?>(null)}
    var epoch by remember{mutableStateOf<Long?>(null)}
    val pdf=rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/pdf")){uri->val copy=ticket;ticket=null;editor.instructionModal(InstructionFocusModal.DocumentPicker,false);if(uri!=null&&copy!=null&&epoch==editor.documentEpoch())editor.saveDocumentCopy(uri,copy);epoch=null}
    val svg=rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("image/svg+xml")){uri->val copy=ticket;ticket=null;editor.instructionModal(InstructionFocusModal.DocumentPicker,false);if(uri!=null&&copy!=null&&epoch==editor.documentEpoch())editor.saveDocumentCopy(uri,copy);epoch=null}
    DisposableEffect(editor){onDispose{editor.instructionModal(InstructionFocusModal.DocumentPicker,false)}}
    DocumentPanel(editor.documents){copy->if(ticket==null&&editor.documents.checkedExport()===copy){ticket=copy;epoch=editor.documentEpoch();editor.instructionModal(InstructionFocusModal.DocumentPicker,true);try{if(copy.mode==DocumentCopy.SvgAnnotations)svg.launch("document-${copy.revision.hostSeq}.svg")else pdf.launch("document-${copy.revision.hostSeq}.pdf")}catch(_:Exception){ticket=null;epoch=null;editor.instructionModal(InstructionFocusModal.DocumentPicker,false)}}}
}
