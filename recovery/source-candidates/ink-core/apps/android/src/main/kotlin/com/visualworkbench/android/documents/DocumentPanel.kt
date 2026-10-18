package com.visualworkbench.android.documents

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable internal fun DocumentPanel(controller:DocumentController,save:(DocumentExport)->Unit) {
    val state by controller.state.collectAsState();val doc=state.descriptor ?: return
    var rasterConsent by remember(doc.sourceAssetId){mutableStateOf(false)};var encryption by remember(doc.sourceAssetId){mutableStateOf(false)};var password by remember(doc.sourceAssetId){mutableStateOf("")}
    Column(Modifier.fillMaxWidth().padding(horizontal=8.dp)) {
        Row(Modifier.horizontalScroll(rememberScrollState())) {
            Text("${doc.kind.name} ${state.page+1u}/${doc.pages.size}",Modifier.padding(12.dp))
            TextButton({controller.page(state.page-1u)},enabled=!state.busy&&state.page>0u){Text("Previous")}
            TextButton({controller.page(state.page+1u)},enabled=!state.busy&&state.page+1u<doc.pages.size.toUInt()){Text("Next")}
            if(doc.kind==EditableKind.Pdf)TextButton({controller.textTool(!state.textTool)},enabled=!state.busy){Text(if(state.textTool)"Annotate" else "Select text")}
            if(doc.kind==EditableKind.Pdf)TextButton({controller.regionTool(!state.regionTool)},enabled=!state.busy){Text(if(state.regionTool)"Stop regions" else "Mark region")}
            TextButton({controller.export(if(doc.kind==EditableKind.Svg)DocumentCopy.SvgAnnotations else DocumentCopy.Annotated)},enabled=!state.busy){Text("Annotated copy")}
            if(doc.kind==EditableKind.Pdf){TextButton({controller.export(DocumentCopy.Original)},enabled=!state.busy){Text("Original")};TextButton({controller.export(DocumentCopy.Flattened)},enabled=!state.busy){Text("Flatten annotations")};TextButton({rasterConsent=true},enabled=!state.busy&&state.regions.isNotEmpty()){Text("Redaction")}}
            controller.checkedExport()?.let{copy->TextButton({save(copy)},enabled=!state.busy){Text("Save ready copy")}}
        }
        if(state.glyphs.isNotEmpty())Row{Text(state.glyphs.joinToString(""){it.text.orEmpty()},Modifier.weight(1f),maxLines=2);TextButton(controller::redactSelection,enabled=!state.busy){Text("Mark for redaction")}}
        state.notices.forEach{Text(documentNoticeText(it),style=MaterialTheme.typography.caption)}
        if(state.passwordNeeded){OutlinedTextField(password,{password=it.take(1024)},label={Text("PDF password")},visualTransformation=androidx.compose.ui.text.input.PasswordVisualTransformation());TextButton({controller.password(password)}){Text("Unlock")}}
        if(state.busy)LinearProgressIndicator(Modifier.fillMaxWidth())
        state.message?.let{Text(it,style=MaterialTheme.typography.caption)}
    }
    if(rasterConsent)AlertDialog(onDismissRequest={rasterConsent=false},title={Text("Rasterize every page for redaction?")},text={Column{Text("This reconstructs every page as pixels, including pages without a marked region. Searchable text, vectors and document features are lost. Ordinary copies never remove underlying content.");Row{Checkbox(encryption,{encryption=it});Text("Allow removal of encryption")}}},confirmButton={TextButton({rasterConsent=false;controller.export(DocumentCopy.EveryPageRasterRedaction,encryption)}){Text("Prepare redaction")}},dismissButton={TextButton({rasterConsent=false}){Text("Keep original")}})
}
