package com.visualworkbench.android.instructions

import android.Manifest
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import com.visualworkbench.shared.*

@Composable
internal fun InstructionsScreen(editor: InstructionEditor, focus: InstructionFocusController, semantic: SemanticEditor, selected: String?, onBack: () -> Unit) {
    val state by editor.state.collectAsState()
    val context = LocalContext.current
    val owner = remember(context) { instructionLifecycleOwner(context) }
    var nativeField by remember { mutableStateOf<InstructionFieldView?>(null) }
    var speechTicket by remember { mutableStateOf<Pair<String, ULong>?>(null) }
    var speechPrefix by remember { mutableStateOf("") }
    var permissionTicket by remember { mutableStateOf<Pair<String, ULong>?>(null) }
    val fieldView = remember { BringIntoViewRequester() }
    val speech = remember(editor) { OnDeviceDictation(context.applicationContext, { text, _ ->
        speechTicket?.let { (id, generation) -> editor.update(speechPrefix + text, id, generation) }
    }, editor::message) }
    fun beginSpeech() {
        editor.method(InstructionEntryMethod.Voice)
        val field = editor.state.value.field ?: return
        speechTicket = field.sessionId to field.generation
        speechPrefix = field.text + if (field.text.isEmpty() || field.text.last().isWhitespace()) "" else " "
        speech.start()
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val expected = permissionTicket; permissionTicket = null
        val field = editor.state.value.field
        if (expected != null && field?.let { it.sessionId to it.generation } == expected && owner.lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) {
            if (granted) beginSpeech() else editor.message("Microphone permission was denied. The keyboard remains available.")
        }
    }
    DisposableEffect(speech, owner) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP) { permissionTicket = null; speechTicket = null; speech.stop() } }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer); permissionTicket = null; speechTicket = null; speech.close() }
    }
    LaunchedEffect(state.field?.sessionId, state.field?.generation, state.busy, state.field?.stale) {
        if (speechTicket != state.field?.let { it.sessionId to it.generation } || state.busy || state.field?.stale == true) { speechTicket = null; speech.stop() }
    }
    LaunchedEffect(state.field?.sessionId) {
        if (state.field != null) { withFrameNanos {}; fieldView.bringIntoView(); nativeField?.requestFocus() }
    }
    Column(Modifier.fillMaxSize().padding(16.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        TextButton(onClick = { speech.stop(); onBack() }, Modifier.heightIn(min = 48.dp)) { Text("← Canvas") }
        Text("Instructions", style = MaterialTheme.typography.h4)
        InstructionFocusControls(focus)
        com.visualworkbench.android.semantics.SemanticPanel(semantic, selected?.takeIf { id -> state.document?.markers?.any { it.objectId == id } == true },
            state.document?.markers?.firstOrNull { it.objectId == selected }?.elementEids.orEmpty())
        TextButton(onClick = { try { context.startActivity(android.content.Intent(context, com.visualworkbench.android.capture.CaptureEntryActivity::class.java)) }
            catch (_: Exception) { editor.message("Capture onboarding could not open. Use Settings to check accessibility capture availability.") } }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Screen capture and accessibility settings") }
        Text("Text stays in this project. Dictation and handwriting fill a draft; Save commits it.")
        Row {
            TextButton(onClick = editor::focusGlobal, enabled = !state.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Global instruction") }
            TextButton(onClick = { selected?.let(editor::focusObject) }, enabled = selected != null && !state.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Selected object") }
        }
        if (state.document?.needsReconciliation == true) Text("Marker links need reconciliation. Saved numbering is preserved.", color = MaterialTheme.colors.error)
        state.document?.markers?.forEach { marker ->
            val row = state.document?.instructions?.firstOrNull { it.instructionId == marker.instructionId }
            OutlinedButton(onClick = { editor.focusObject(marker.objectId) }, enabled = !state.busy, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                Text("${marker.number}. ${row?.text?.ifBlank { "Add instruction" } ?: "Missing instruction"}", maxLines = 3)
            }
        }
        state.document?.instructions?.filter { row -> state.document?.markers?.none { it.instructionId == row.instructionId } == true }?.forEach { row ->
            TextButton(onClick = { editor.focusInstruction(row.instructionId) }, enabled = !state.busy, modifier = Modifier.heightIn(min = 48.dp)) {
                Text(if (row.targetIds.isEmpty()) "Global instruction" else if (row.detached) "Detached instruction · repair required" else "Object instruction")
            }
        }
        state.field?.let { field ->
            Text(if (field.targetIds.isEmpty()) "Global" else "Focused instruction", style = MaterialTheme.typography.h6)
            RoleChoices(field.role, !state.busy, editor::role)
            AndroidView(factory = { InstructionFieldView(it).also { view -> nativeField = view } }, update = { view ->
                view.unavailable = editor::message
                view.changed = { text ->
                    speechTicket = null; speech.stop()
                    editor.update(text, field.sessionId, field.generation)
                }
                view.apply(field.text, field.method, !state.busy)
            }, modifier = Modifier.fillMaxWidth().heightIn(min = 150.dp).bringIntoViewRequester(fieldView))
            Text("Entry: ${field.method.name}", style = MaterialTheme.typography.caption)
            Row {
                TextButton(onClick = { speech.stop(); editor.method(InstructionEntryMethod.PhoneKeyboard); nativeField?.requestFocus() }, enabled = !state.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Keyboard") }
                TextButton(onClick = {
                    if (!speech.available()) editor.message("On-device dictation is unavailable. Enable an on-device speech provider and language model in Android settings, or use Keyboard.")
                    else if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) beginSpeech()
                    else { permissionTicket = field.sessionId to field.generation; permission.launch(Manifest.permission.RECORD_AUDIO) }
                }, enabled = !state.busy && !field.stale && permissionTicket == null, modifier = Modifier.heightIn(min = 48.dp)) { Text("Dictate") }
                TextButton(onClick = { speechTicket = null; speech.stop() }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Stop") }
            }
            TextButton(onClick = {
                speech.stop()
                if (nativeField?.handwritingAvailable() == true) { editor.method(InstructionEntryMethod.Handwriting); nativeField?.requestFocus(); editor.message("Write in the instruction field with your stylus. The keyboard's handwriting provider supplies the text.") }
                else editor.message("Stylus handwriting requires Android 14+ and a supporting active keyboard. Enable it in keyboard settings, or use Keyboard. Samsung-specific recognition has not been integrated.")
            }, enabled = !state.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Handwriting") }
            if (field.stale) { Text("Document changed. Your complete draft is retained."); TextButton(onClick = editor::reapply, enabled = !state.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Refresh and reapply draft") } }
            Row {
                Button(onClick = { speech.stop(); editor.save() }, enabled = field.dirty && !field.stale && !state.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Save") }
                TextButton(onClick = { speech.stop(); editor.discard() }, enabled = !state.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Discard draft") }
                if (state.busy) TextButton(onClick = { speech.stop(); editor.cancel() }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel") }
            }
            val marker = state.document?.markers?.firstOrNull { it.instructionId == field.instructionId }
            TextButton(onClick = { if (marker != null) editor.deleteMarker(marker.objectId) else editor.deleteInstruction(field.instructionId) },
                enabled = !state.busy && !field.dirty && field.existing, modifier = Modifier.heightIn(min = 48.dp)) { Text(if (marker != null) "Delete marker and instruction" else "Delete instruction") }
        }
        if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        state.message?.let { Text(it, color = MaterialTheme.colors.secondary) }
    }
}
private fun instructionLifecycleOwner(context: Context): LifecycleOwner {
    var current = context
    repeat(16) {
        if (current is LifecycleOwner) return current as LifecycleOwner
        val next = (current as? ContextWrapper)?.baseContext
            ?: error("Instruction screen requires an Activity lifecycle")
        check(next !== current)
        current = next
    }
    error("Instruction lifecycle wrapper limit")
}
@Composable private fun RoleChoices(value: InstructionRole, enabled: Boolean, change: (InstructionRole) -> Unit) {
    Column {
        Text("Role · independent of color")
        InstructionRole.entries.forEach { role ->
            Row {
                RadioButton(value == role, { change(role) }, enabled = enabled)
                TextButton({ change(role) }, enabled = enabled, modifier = Modifier.heightIn(min = 48.dp)) {
                    Text(role.name)
                }
            }
        }
    }
}
