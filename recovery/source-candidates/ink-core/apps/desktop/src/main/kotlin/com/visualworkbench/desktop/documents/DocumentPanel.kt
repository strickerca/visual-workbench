package com.visualworkbench.desktop.documents

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable internal fun DocumentPanel(controller:DocumentController,save:(DocumentExport)->Unit) {
    val state by controller.state.collectAsState();val doc=state.descriptor ?: return
    var mode by remember(doc.kind){mutableStateOf(if(doc.kind==EditableKind.Svg)DocumentCopy.SvgAnnotations else DocumentCopy.Annotated)}
    var encryption by remember(doc.sourceAssetId){mutableStateOf(false)};var password by remember(doc.sourceAssetId){mutableStateOf("")}
    Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(6.dp)) {
        Text(if(doc.kind==EditableKind.Pdf)"PDF · page ${state.page+1u} of ${doc.pages.size}" else "SVG",style=MaterialTheme.typography.subtitle2)
        if(doc.pages.size>1)Row{TextButton({controller.page(state.page-1u)},enabled=!state.busy&&state.page>0u){Text("Previous")};TextButton({controller.page(state.page+1u)},enabled=!state.busy&&state.page+1u<doc.pages.size.toUInt()){Text("Next")}}
        if(doc.kind==EditableKind.Pdf) {
            TextButton({controller.textTool(!state.textTool)},enabled=!state.busy){Text(if(state.textTool)"Return to annotation tools" else "Select PDF text")}
            TextButton({controller.regionTool(!state.regionTool)},enabled=!state.busy){Text(if(state.regionTool)"Stop marking regions" else "Mark redaction region")}
            if(state.glyphs.isNotEmpty()) {Text(state.glyphs.joinToString(""){it.text.orEmpty()},style=MaterialTheme.typography.body2);TextButton(controller::redactSelection,enabled=!state.busy){Text("Mark selected text for redaction")}}
            listOf(DocumentCopy.Original to "Original PDF",DocumentCopy.Annotated to "PDF with annotations",DocumentCopy.Flattened to "Flattened annotations",DocumentCopy.PcRedaction to "PC object removal",DocumentCopy.EveryPageRasterRedaction to "Rasterize every page").forEach{(choice,label)->Row{RadioButton(mode==choice,{mode=choice},enabled=!state.busy);Text(label,Modifier.padding(top=12.dp))}}
            if(mode in setOf(DocumentCopy.PcRedaction,DocumentCopy.EveryPageRasterRedaction)) {
                Text("${state.regions.size} marked regions. PC removal refuses unsupported unaffected content. Rasterization removes searchable text and vector features on every page.",style=MaterialTheme.typography.caption)
                Row{Checkbox(encryption,{encryption=it});Text("Allow removal of encryption",Modifier.padding(top=12.dp))}
                TextButton(controller::clearRegions,enabled=!state.busy){Text("Clear redaction marks")}
            }
        }
        if(state.passwordNeeded){OutlinedTextField(password,{password=it.take(1024)},label={Text("PDF password")},visualTransformation=androidx.compose.ui.text.input.PasswordVisualTransformation());TextButton({controller.password(password)}){Text("Unlock")}}
        state.notices.forEach{Text(documentNoticeText(it),style=MaterialTheme.typography.caption)}
        Button({controller.export(mode,encryption)},enabled=!state.busy&&(mode !in setOf(DocumentCopy.PcRedaction,DocumentCopy.EveryPageRasterRedaction)||state.regions.isNotEmpty())){Text("Prepare document export")}
        controller.checkedExport()?.let{copy->Button({save(copy)},enabled=!state.busy){Text("Save prepared ${if(copy.redacted)"redaction" else "copy"}")}}
        if(state.busy)LinearProgressIndicator(Modifier.fillMaxWidth())
        state.message?.let{Text(it,style=MaterialTheme.typography.caption)}
    }
}
