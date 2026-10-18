package @PACKAGE@

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable
internal fun ImageOutputPanel(controls: ImageOutputInteraction, activate: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val state by controls.state.collectAsState()
    OutlinedButton({ activate(true) }, modifier=modifier.heightIn(min=48.dp)) {
        Text(if(state.busy) "Image output · saving…" else if(state.retry!=null) "Image output · retry retained" else "Image output")
    }
    if(!state.active)return
    val snapshot=state.snapshot
    val form=state.form
    val locked=when(form){is ImageOutputForm.Frame->snapshot?.output?.locked==true
        is ImageOutputForm.Tone->snapshot?.tones?.firstOrNull{it.objectId==form.objectId}?.locked==true;null->false}
    val editable=state.ready && !locked
    var previewX by remember(snapshot?.binding){mutableStateOf("0")}
    var previewY by remember(snapshot?.binding){mutableStateOf("0")}
    var marked by remember{mutableStateOf(true)}
    var assume by remember{mutableStateOf(false)}
    AlertDialog(onDismissRequest={activate(false)},title={Text("Image output")},text={
        Column(Modifier.fillMaxWidth().heightIn(min=240.dp,max=600.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            Text("Crop, rotate, resize and tone changes are nondestructive. Original pixels and editing coordinates stay unchanged. Saved exports use these output settings.",style=MaterialTheme.typography.body2)
            snapshot?.let{Text("Original: ${it.sourceWidth} × ${it.sourceHeight} · ${it.sourceBitDepth}-bit. Committed output: ${it.outputWidth} × ${it.outputHeight}.",style=MaterialTheme.typography.caption)}
            if(state.loading || state.busy)LinearProgressIndicator(Modifier.fillMaxWidth())
            if(state.stale)Text("The visible revision changed. Your private values are retained. Discard them and reload before applying.")
            if(locked)Text("This saved setting is locked. Unlock it in Documents and layers to edit it.")
            state.issue?.let{Text(it,color=MaterialTheme.colors.secondary)}
            state.lastReceipt?.let{Text("Saved at revision ${it.revision.hostSeq}. Use Undo in the editor to restore the prior setting.",style=MaterialTheme.typography.caption)}
            if(state.retry!=null) {
                Text("One exact edit is retained. Retry learns whether it was saved; discard does not undo an edit already in history.")
                Row {
                    TextButton({controls.retry()},enabled=!state.busy){Text("Retry exact edit")}
                    TextButton({controls.discardRetry()},enabled=!state.busy){Text("Discard retry")}
                }
            }
            if(state.busy)TextButton(controls::cancel){Text("Cancel and settle")}
            TextButton({controls.discardFormAndReload()},enabled=!state.busy && state.retry==null){Text(if(state.dirty)"Discard form changes and reload" else "Reload saved settings")}
            Row {
                TextButton({controls.selectFrame()},enabled=state.ready && !state.dirty){Text("Output frame")}
                TextButton({controls.selectTone(null)},enabled=state.ready && !state.dirty){Text("New tone layer")}
            }
            snapshot?.tones?.forEachIndexed{index,tone->
                TextButton({controls.selectTone(tone.objectId)},enabled=state.ready && !state.dirty,modifier=Modifier.fillMaxWidth()) {
                    Text("Tone ${index+1} · ${if(tone.enabled)"enabled" else "disabled"} · ${(tone.opacity*100).toInt()}%${if(tone.locked)" · locked" else ""}")
                }
            }
            when(form) {
                is ImageOutputForm.Frame -> {
                    Text("Output frame",style=MaterialTheme.typography.subtitle1)
                    Text("Crop edges use whole original pixels. Ordinary crop guides are unchanged.",style=MaterialTheme.typography.caption)
                    OutputToggle("Enable this output frame",form.enabled,editable){controls.edit(form.copy(enabled=it))}
                    Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        OutputField("Crop left",form.cropX,editable,Modifier.weight(1f)){controls.edit(form.copy(cropX=it))}
                        OutputField("Crop top",form.cropY,editable,Modifier.weight(1f)){controls.edit(form.copy(cropY=it))}
                    }
                    Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        OutputField("Crop width",form.cropWidth,editable,Modifier.weight(1f)){controls.edit(form.copy(cropWidth=it))}
                        OutputField("Crop height",form.cropHeight,editable,Modifier.weight(1f)){controls.edit(form.copy(cropHeight=it))}
                    }
                    Text("Clockwise output rotation")
                    Row{listOf(0,90,180,270).forEach{angle->TextButton({controls.edit(form.copy(clockwiseDegrees=angle.toUShort()))},enabled=editable){Text(if(form.clockwiseDegrees.toInt()==angle)"✓ $angle°" else "$angle°")}}}
                    Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        OutputField("Output width",form.outputWidth,editable,Modifier.weight(1f)){controls.edit(form.copy(outputWidth=it))}
                        OutputField("Output height",form.outputHeight,editable,Modifier.weight(1f)){controls.edit(form.copy(outputHeight=it))}
                    }
                    TextButton({val quarter=form.clockwiseDegrees.toInt() in listOf(90,270);controls.edit(form.copy(outputWidth=if(quarter)form.cropHeight else form.cropWidth,outputHeight=if(quarter)form.cropWidth else form.cropHeight))},enabled=editable){Text("Use rotated crop dimensions")}
                    snapshot?.let{source->TextButton({controls.edit(ImageOutputForm.Frame("0","0",source.sourceWidth.toString(),source.sourceHeight.toString(),0u,source.sourceWidth.toString(),source.sourceHeight.toString(),form.enabled))},enabled=editable){Text("Reset private frame to original")}}
                    Text("Width and height resize independently. Resampling uses premultiplied bilinear samples in the source color space. Each side is at most 32,768 px and output at most 50 MP; memory can require a smaller operation. No implicit downsampling.",style=MaterialTheme.typography.caption)
                    Button({controls.apply()},enabled=editable,modifier=Modifier.fillMaxWidth().heightIn(min=48.dp)){Text("Apply output frame")}
                    TextButton({controls.remove()},enabled=editable && !state.dirty && snapshot?.output!=null){Text("Remove saved output frame")}
                }
                is ImageOutputForm.Tone -> {
                    Text(if(form.objectId==null)"New tone layer" else "Saved tone layer",style=MaterialTheme.typography.subtitle1)
                    Text("Brightness and contrast: −1 to 1. Black/white levels and opacity: 0 to 1. Input black must be less than white; gamma must be positive.",style=MaterialTheme.typography.caption)
                    OutputToggle("Enable this tone layer",form.enabled,editable){controls.edit(form.copy(enabled=it))}
                    Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        OutputField("Brightness",form.brightness,editable,Modifier.weight(1f)){controls.edit(form.copy(brightness=it))}
                        OutputField("Contrast",form.contrast,editable,Modifier.weight(1f)){controls.edit(form.copy(contrast=it))}
                    }
                    Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        OutputField("Input black",form.inputBlack,editable,Modifier.weight(1f)){controls.edit(form.copy(inputBlack=it))}
                        OutputField("Input white",form.inputWhite,editable,Modifier.weight(1f)){controls.edit(form.copy(inputWhite=it))}
                    }
                    OutputField("Gamma",form.gamma,editable){controls.edit(form.copy(gamma=it))}
                    Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        OutputField("Output black",form.outputBlack,editable,Modifier.weight(1f)){controls.edit(form.copy(outputBlack=it))}
                        OutputField("Output white",form.outputWhite,editable,Modifier.weight(1f)){controls.edit(form.copy(outputWhite=it))}
                    }
                    OutputField("Layer opacity",form.opacity,editable){controls.edit(form.copy(opacity=it))}
                    TextButton({controls.edit(ImageOutputForm.Tone(form.objectId,enabled=form.enabled))},enabled=editable){Text("Reset private tone values")}
                    Button({controls.apply()},enabled=editable,modifier=Modifier.fillMaxWidth().heightIn(min=48.dp)){Text("Apply tone layer")}
                    TextButton({controls.remove()},enabled=editable && !state.dirty && form.objectId!=null){Text("Remove saved tone layer")}
                }
                null -> Unit
            }
            if(state.dirty)Text("These form values are private and unsaved. Apply or discard them before closing this panel.",style=MaterialTheme.typography.caption)
            Divider()
            Text("Committed output preview",style=MaterialTheme.typography.subtitle1)
            Text("One exact region, up to 512 × 512 output pixels. Apply first to inspect changed settings. This display uses 8-bit sRGB; export still uses your chosen depth and color options.",style=MaterialTheme.typography.caption)
            Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                OutputField("Preview left",previewX,!state.busy,Modifier.weight(1f)){previewX=it}
                OutputField("Preview top",previewY,!state.busy,Modifier.weight(1f)){previewY=it}
            }
            OutputToggle("Preview marked output",marked,!state.busy){marked=it}
            OutputToggle("Treat untagged source colors as sRGB for this preview",assume,!state.busy){assume=it}
            OutlinedButton({val x=previewX.toUIntOrNull();val y=previewY.toUIntOrNull();if(x!=null && y!=null)controls.preview(x,y,marked,assume)},
                enabled=state.ready && previewX.toUIntOrNull()!=null && previewY.toUIntOrNull()!=null,modifier=Modifier.heightIn(min=48.dp)){Text("Show committed region")}
            if(state.previewLoading)LinearProgressIndicator(Modifier.fillMaxWidth())
            state.preview?.let{preview->
                Text("Committed revision ${preview.binding.hostSeq} · ${if(state.previewMarked)"marked" else "clean"} · output region [${preview.x}, ${preview.y}, ${preview.width}, ${preview.height}]",style=MaterialTheme.typography.caption)
                ImageOutputPreviewTile(preview)
            }
        }
    },confirmButton={TextButton({activate(false)},enabled=state.mayLeave){Text("Close")}})
}

@Composable
private fun OutputField(label:String,value:String,enabled:Boolean,modifier:Modifier=Modifier,change:(String)->Unit) {
    OutlinedTextField(value,{if(it.length<=32)change(it)},label={Text(label)},enabled=enabled,singleLine=true,modifier=modifier)
}
@Composable
private fun OutputToggle(label:String,value:Boolean,enabled:Boolean,change:(Boolean)->Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min=48.dp),verticalAlignment=Alignment.CenterVertically){Checkbox(value,change,enabled=enabled);Text(label,Modifier.weight(1f),style=MaterialTheme.typography.body2)}
}
