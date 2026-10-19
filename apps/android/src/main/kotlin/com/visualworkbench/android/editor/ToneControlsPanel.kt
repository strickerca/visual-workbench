package com.visualworkbench.android.editor

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

/** Private drafts preview without committing; Apply/Remove are explicit actions.
 * Canvas composition is a separate caller hook and is not needed by this panel. */
@Composable
internal fun ToneControlsPanel(controls: ToneControlsInteraction, activate: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val state by controls.state.collectAsState()
    val opening = controls.capture()
    OutlinedButton(onClick={if (opening != null && controls.capture() == opening) activate(true)},
        enabled=opening != null,modifier=modifier.heightIn(min=48.dp)) {
        Text(if (state.busy) "Tone controls · saving…" else if (state.retry != null) "Tone controls · retry retained" else "Tone controls")
    }
    if (!state.active) return
    val token = controls.capture()
    val snapshot = state.snapshot
    val form = state.form
    val locked = snapshot?.tones?.any { it.objectId == form?.objectId && it.locked } == true
    val editable = state.ready && token != null && !locked
    var previewX by remember(token) { mutableStateOf("0") }
    var previewY by remember(token) { mutableStateOf("0") }
    var draft by remember(token) { mutableStateOf(true) }
    var assume by remember(token) { mutableStateOf(false) }
    fun current(): Boolean = token != null && controls.capture() == token
    fun change(update: (ToneControlForm) -> ToneControlForm) {
        if (token == null || !current()) return
        val latest = controls.state.value.form ?: return
        controls.edit(token,update(latest))
    }
    fun local(change: () -> Unit) {
        if (current()) { change(); token?.let { controls.invalidatePreview(it) } }
    }
    val x = toneCoordinate(previewX); val y = toneCoordinate(previewY)
    AlertDialog(onDismissRequest={if (current()) activate(false)},title={Text("Tone controls")},text={
        Column(Modifier.fillMaxWidth().heightIn(min=240.dp,max=600.dp).verticalScroll(rememberScrollState()),
            verticalArrangement=Arrangement.spacedBy(8.dp)) {
            Text("Brightness, contrast and levels are saved as editable parameters. The original pixels remain intact.",
                style=MaterialTheme.typography.body2)
            snapshot?.let { Text("Original: ${it.sourceWidth} × ${it.sourceHeight} · ${it.sourceBitDepth}-bit.",
                style=MaterialTheme.typography.caption) }
            if (state.loading || state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (state.stale) Text("The visible revision changed. Private values are retained; discard them and reload before applying.")
            if (locked) Text("This saved tone is locked. Unlock its object and layer in Documents and layers before editing it.")
            state.issue?.let { Text(it,color=MaterialTheme.colors.secondary) }
            state.lastReceipt?.let { Text("Saved at revision ${it.revision.hostSeq}. Undo in the editor restores the prior parameters.",
                style=MaterialTheme.typography.caption) }
            if (state.retry != null) {
                Text("One exact edit is retained. Retry resolves whether it was saved; discarding the handle leaves saved history intact.")
                Row {
                    TextButton(onClick={token?.let { controls.retry(it) }},enabled=!state.busy && token != null) { Text("Retry exact edit") }
                    TextButton(onClick={token?.let { controls.discardRetry(it) }},enabled=!state.busy && token != null) { Text("Discard retry") }
                }
            }
            if (state.busy) TextButton(onClick={token?.let { controls.cancel(it) }},enabled=token != null) { Text("Cancel and settle") }
            TextButton(onClick={token?.let { controls.discardFormAndReload(it) }},
                enabled=!state.busy && state.retry == null && token != null) {
                Text(if (state.dirty) "Discard private values and reload" else "Reload saved tones")
            }
            TextButton(onClick={token?.let { controls.selectTone(it,null) }},enabled=state.ready && !state.dirty && token != null) {
                Text(if (form?.objectId == null) "Selected · new tone layer" else "New tone layer")
            }
            snapshot?.tones?.forEachIndexed { index,tone ->
                TextButton(onClick={token?.let { controls.selectTone(it,tone.objectId) }},
                    enabled=state.ready && !state.dirty && token != null,modifier=Modifier.fillMaxWidth()) {
                    Text((if (tone.objectId == form?.objectId) "Selected · " else "") + "Tone ${index+1} · " +
                        (if (tone.enabled) "enabled" else "disabled") + " · ${(tone.opacity*100).toInt()}%" +
                        (if (tone.locked) " · locked" else ""))
                }
            }
            if (form != null) {
                Text(if (form.objectId == null) "New tone layer" else "Saved tone layer",style=MaterialTheme.typography.subtitle1)
                Text("Brightness and contrast: −1 to 1. Levels and opacity: 0 to 1. Input black must be below white; output black may equal white. Gamma must be positive.",
                    style=MaterialTheme.typography.caption)
                ToneToggle("Enable this tone layer",form.enabled,editable) { value -> change { it.copy(enabled=value) } }
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    ToneField("Brightness",form.brightness,editable,Modifier.weight(1f)) { value -> change { it.copy(brightness=value) } }
                    ToneField("Contrast",form.contrast,editable,Modifier.weight(1f)) { value -> change { it.copy(contrast=value) } }
                }
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    ToneField("Input black",form.inputBlack,editable,Modifier.weight(1f)) { value -> change { it.copy(inputBlack=value) } }
                    ToneField("Input white",form.inputWhite,editable,Modifier.weight(1f)) { value -> change { it.copy(inputWhite=value) } }
                }
                ToneField("Gamma",form.gamma,editable) { value -> change { it.copy(gamma=value) } }
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    ToneField("Output black",form.outputBlack,editable,Modifier.weight(1f)) { value -> change { it.copy(outputBlack=value) } }
                    ToneField("Output white",form.outputWhite,editable,Modifier.weight(1f)) { value -> change { it.copy(outputWhite=value) } }
                }
                ToneField("Layer opacity",form.opacity,editable) { value -> change { it.copy(opacity=value) } }
                TextButton(onClick={change { ToneControlForm(it.objectId,enabled=it.enabled) }},enabled=editable) { Text("Reset private tone values") }
                Button(onClick={token?.let { controls.apply(it) }},enabled=editable,
                    modifier=Modifier.fillMaxWidth().heightIn(min=48.dp)) { Text("Apply tone layer") }
                TextButton(onClick={token?.let { controls.remove(it) }},
                    enabled=editable && !state.dirty && form.objectId != null) { Text("Remove saved tone layer") }
            }
            if (state.dirty) Text("These private values are unsaved. Apply or discard them before closing.",style=MaterialTheme.typography.caption)
            Divider()
            Text("Tone preview",style=MaterialTheme.typography.subtitle1)
            Text("One exact region of up to 512 × 512 original pixels, displayed as 8-bit sRGB. Private preview evaluates these values without saving them. Larger work may exceed the memory budget.",
                style=MaterialTheme.typography.caption)
            Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                ToneField("Preview left",previewX,state.ready && token != null,Modifier.weight(1f)) { value -> local { previewX=value } }
                ToneField("Preview top",previewY,state.ready && token != null,Modifier.weight(1f)) { value -> local { previewY=value } }
            }
            ToneToggle("Preview private values",draft,state.ready && token != null) { value -> local { draft=value } }
            ToneToggle("Treat untagged source colors as sRGB for this preview",assume,state.ready && token != null) { value -> local { assume=value } }
            OutlinedButton(onClick={if (token != null && x != null && y != null) controls.preview(token,x,y,assume,draft)},
                enabled=state.ready && token != null && x != null && y != null && (!draft || !locked),
                modifier=Modifier.heightIn(min=48.dp)) { Text(if (draft) "Preview private values" else "Preview saved tones") }
            if (state.previewLoading) LinearProgressIndicator(Modifier.fillMaxWidth())
            state.preview?.let { preview ->
                Text((if (state.previewDraft) "Private preview" else "Saved tones") + " · revision ${preview.binding.hostSeq} · " +
                    "region [${preview.x}, ${preview.y}, ${preview.width}, ${preview.height}]",style=MaterialTheme.typography.caption)
                TonePreviewTile(preview)
            }
        }
    },confirmButton={TextButton(onClick={if (current()) activate(false)},enabled=state.mayLeave && token != null) { Text("Close") }})
}

@Composable
private fun ToneField(label: String, value: String, enabled: Boolean, modifier: Modifier = Modifier, change: (String) -> Unit) {
    OutlinedTextField(value,{if (it.length <= 32) change(it)},label={Text(label)},enabled=enabled,singleLine=true,modifier=modifier)
}
@Composable
private fun ToneToggle(label: String, value: Boolean, enabled: Boolean, change: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min=48.dp),verticalAlignment=Alignment.CenterVertically) {
        Checkbox(value,change,enabled=enabled); Text(label,Modifier.weight(1f),style=MaterialTheme.typography.body2)
    }
}
private fun toneCoordinate(text: String): UInt? =
    text.takeIf { it.isNotEmpty() && it.all { value -> value in '0'..'9' } }?.toUIntOrNull()
