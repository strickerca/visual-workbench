package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable internal fun EditableDocumentControls(controller:DocumentController,save:(DocumentExport)->Unit) {
    val state by controller.state.collectAsState()
    val descriptor=state.descriptor ?: return
    var password by remember(descriptor.sourceAssetId){mutableStateOf("")}
    var mode by remember(descriptor.sourceAssetId){mutableStateOf(if(descriptor.kind==EditableKind.Svg)DocumentCopy.SvgAnnotations else DocumentCopy.Annotated)}
    var removeEncryption by remember(descriptor.sourceAssetId){mutableStateOf(false)}
    Column(Modifier.fillMaxWidth().padding(8.dp),verticalArrangement=Arrangement.spacedBy(6.dp)){
        Text(if(descriptor.kind==EditableKind.Pdf)"PDF · page ${state.page+1u} of ${descriptor.pages.size}" else "SVG document")
        if(descriptor.pages.size>1)Row{
            TextButton({controller.page(state.page-1u)},enabled=!state.busy&&state.page>0u){Text("Previous page")}
            TextButton({controller.page(state.page+1u)},enabled=!state.busy&&state.page+1u<descriptor.pages.size.toUInt()){Text("Next page")}
        }
        if(descriptor.kind==EditableKind.Pdf){
            TextButton({controller.textTool(!state.textTool)},enabled=!state.busy){Text(if(state.textTool)"Stop selecting text" else "Select PDF text")}
            TextButton({controller.regionTool(!state.regionTool)},enabled=!state.busy){Text(if(state.regionTool)"Stop marking regions" else "Mark redaction regions")}
            if(state.textTool||state.regionTool)Text("Drag a rectangle on the page.",style=MaterialTheme.typography.caption)
            if(state.glyphs.isNotEmpty()){
                Text(state.glyphs.joinToString(""){it.text.orEmpty()},style=MaterialTheme.typography.body2)
                TextButton(controller::redactSelection,enabled=!state.busy){Text("Mark selected text for redaction")}
            }
            listOf(DocumentCopy.Original to "Original PDF",DocumentCopy.Annotated to "PDF with annotations",DocumentCopy.Flattened to "Flattened annotations",DocumentCopy.PcRedaction to "PC object removal",DocumentCopy.EveryPageRasterRedaction to "Rasterize every page").forEach{(choice,label)->
                Row{RadioButton(mode==choice,{mode=choice},enabled=!state.busy&&controller.canExport(choice)&&(choice!=DocumentCopy.PcRedaction||controller.pcRedaction));Text(label,Modifier.padding(top=12.dp))}
            }
            if(mode==DocumentCopy.PcRedaction||mode==DocumentCopy.EveryPageRasterRedaction){
                Text("${state.regions.size} marked regions. Full-page raster redaction removes searchable text and vector features on every output page.",style=MaterialTheme.typography.caption)
                Row{Checkbox(removeEncryption,{removeEncryption=it},enabled=!state.busy);Text("Allow removal of encryption")}
                TextButton(controller::clearRegions,enabled=!state.busy){Text("Clear regions")}
            }
        }
        if(state.passwordNeeded){
            OutlinedTextField(password,{password=it.take(1024)},label={Text("PDF password")},visualTransformation=PasswordVisualTransformation(),enabled=!state.busy)
            TextButton({controller.password(password);password=""},enabled=!state.busy){Text("Unlock")}
        }
        state.notices.forEach{Text(documentNoticeText(it),style=MaterialTheme.typography.caption)}
        TextButton(controller::retryViewport,enabled=!state.busy){Text("Retry page display")}
        Button({controller.export(mode,removeEncryption)},enabled=!state.busy&&controller.canExport(mode)){Text("Prepare document copy")}
        if(!controller.canExport(mode))Text(controller.copyUnavailableText(mode),style=MaterialTheme.typography.caption)
        controller.checkedExport()?.let{copy->Button({save(copy)},enabled=!state.busy){Text("Save prepared copy")}}
        if(state.busy){LinearProgressIndicator(Modifier.fillMaxWidth());TextButton(controller::cancel){Text("Cancel document work")}}
        state.message?.let{Text(it,style=MaterialTheme.typography.caption)}
    }
}
