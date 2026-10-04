package com.visualworkbench.android.ui

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.result.PickVisualMediaRequest
import android.app.Activity
import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.visualworkbench.android.editor.*
import com.visualworkbench.shared.Shape
import com.visualworkbench.android.instructions.InstructionsScreen
import com.visualworkbench.android.instructions.InstructionFocusModal
import com.visualworkbench.shared.SelectionSaveTicket

@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
@Composable
internal fun WorkbenchApp(editor: EditorController, onCamera: () -> Unit = {}) {
    val context = LocalContext.current
    var showExport by remember { mutableStateOf(false) }
    fun exportDialog(value: Boolean) { editor.instructionModal(InstructionFocusModal.ExportDialog, value); showExport = value }
    // Deliberately not saveable across process death: without its live project
    // attachment, a returned destination is discarded rather than rebound.
    var selectionPicker by remember { mutableStateOf<SelectionSaveTicket?>(null) }
    val selectionExporter = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("image/png")) { uri ->
        val ticket = selectionPicker; selectionPicker = null
        if (uri != null && (ticket == null || !editor.selections.save(ticket, uri))) editor.selections.discardUnused(uri)
    }
    fun saveSelection(ticket: SelectionSaveTicket) {
        if (selectionPicker != null) return
        selectionPicker = ticket
        try { selectionExporter.launch("selection-${ticket.kind.name.lowercase()}-${ticket.binding.hostSeq}.png") }
        catch (_: Exception) { selectionPicker = null; editor.transferMessage("The PNG destination picker is unavailable.") }
    }
    var pickerTicketId by rememberSaveable { mutableStateOf<String?>(null) }
    var cameraSaveToken by rememberSaveable { mutableStateOf<String?>(null) }
    var cameraDiscardToken by remember { mutableStateOf<String?>(null) }
    val imagePicker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uri?.let { editor.receiveImage(it) } }
    val photoPicker = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri -> uri?.let { editor.receiveImage(it) } }
    val exporter = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        try {
            val uri = result.data?.data
            if (result.resultCode == Activity.RESULT_OK && uri != null) editor.export(uri, pickerTicketId)
            else editor.cancelExportTicket(pickerTicketId)
        } finally {
            pickerTicketId = null
            editor.instructionModal(InstructionFocusModal.ExportPicker, false)
        }
    }
    val cameraSaver = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri ->
        val token = cameraSaveToken
        cameraSaveToken = null
        if (uri != null && token != null) editor.saveCamera(token, uri)
    }
    DisposableEffect(editor) {
        // Restore the guard for a retained ActivityResult ticket before this
        // composition returns control to the UI event loop.
        editor.instructionModal(InstructionFocusModal.ExportPicker, pickerTicketId != null)
        onDispose {
            editor.instructionFocus.sharing(false); editor.instructionFocus.following(false)
            editor.clearInstructionModals()
        }
    }
    LaunchedEffect(editor.textDraft != null, editor.showBrush, editor.showContext, editor.blockingMessage != null) {
        editor.instructionInteractionChanged()
    }
    LaunchedEffect(editor.shareUri) {
        val uri = editor.consumeShare() ?: return@LaunchedEffect
        try { context.startActivity(Intent.createChooser(MediaIntents.sharePng(uri), "Share PNG")) }
        catch (_: Exception) { editor.shareLaunchFailed(uri) }
    }
    BackHandler(editor.screen != WorkbenchScreen.Projects) {
        if(editor.ai.state.value.open)editor.hideAi()
        else if (editor.textDraft != null) editor.dismissText()
        else if (editor.showBrush) editor.brushDialog(false)
        else if (editor.screen == WorkbenchScreen.Settings) editor.returnFromSettings()
        else if (editor.screen == WorkbenchScreen.Instructions) editor.navigate(WorkbenchScreen.Canvas)
        else if (editor.screen == WorkbenchScreen.Pairing && editor.document != null) editor.navigate(WorkbenchScreen.Canvas)
        else editor.navigate(WorkbenchScreen.Projects)
    }
    Surface(Modifier.fillMaxSize().safeDrawingPadding().semantics {
        testTagsAsResourceId = (context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0
    }, color = MaterialTheme.colors.background) {
        Column {
            val captureStatus by editor.agentCapture.state.collectAsState()
            AgentCaptureIndicator(captureStatus)
            ReceivedCaptureBanner(editor)
            Box(Modifier.weight(1f)) {
            when (editor.screen) {
                WorkbenchScreen.Projects -> ProjectsScreen(editor.projects, editor.busy,
                    { photoPicker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) },
                    { editor.newCanvas() }, { editor.open(it) },
                    { editor.navigate(WorkbenchScreen.Settings) }, { editor.navigate(WorkbenchScreen.Diagnostics) }, { editor.navigate(WorkbenchScreen.Pairing) },
                    { imagePicker.launch(arrayOf("image/png", "image/jpeg", "image/webp", "image/heic", "image/heif")) }, onCamera,
                    editor.cameraCaptures, editor::retryCamera, { cameraDiscardToken = it }, { token ->
                        if (cameraSaveToken != null) editor.transferMessage("Finish or cancel the pending original-file destination first.")
                        else {
                            cameraSaveToken = token
                            try { cameraSaver.launch("camera-original") }
                            catch (_: Exception) { cameraSaveToken = null; editor.transferMessage("A file destination is unavailable on this device.") }
                        }
                    })
                WorkbenchScreen.Canvas -> EditorScreen(editor, { exportDialog(true) }, ::saveSelection)
                WorkbenchScreen.Settings -> Column {
                    TextButton(onClick = { context.startActivity(Intent(context, com.visualworkbench.android.capture.CaptureEntryActivity::class.java)) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Screen capture and accessibility") }
                    Box(Modifier.weight(1f)) { SettingsScreen(editor.preferences, editor::settings, editor::returnFromSettings) }
                }
                WorkbenchScreen.Diagnostics -> DiagnosticsScreen(editor.diagnostics) { editor.navigate(WorkbenchScreen.Projects) }
                WorkbenchScreen.Pairing -> PairingScreen(editor)
                WorkbenchScreen.Instructions -> InstructionsScreen(editor.instructionEditor, editor.instructionFocus, editor.semanticEditor, editor.selected) { editor.navigate(WorkbenchScreen.Canvas) }
            }
            if (editor.transferLabel != null && editor.screen != WorkbenchScreen.Canvas) {
                Surface(Modifier.align(Alignment.Center).padding(24.dp), shape = RoundedCornerShape(16.dp), elevation = 12.dp) {
                    Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text(checkNotNull(editor.transferLabel))
                        LinearProgressIndicator(Modifier.fillMaxWidth())
                        TextButton(onClick = editor::cancelTransfer, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel transfer") }
                    }
                }
            }
            editor.message?.let { message ->
                Snackbar(Modifier.align(Alignment.BottomCenter).padding(12.dp), action = {
                    TextButton(onClick = editor::dismissMessage, modifier = Modifier.heightIn(min = 48.dp)) { Text("Dismiss", color = MaterialTheme.colors.primary) }
                }) { Text(message) }
            }
        }
        }
    }
    AiEditorOverlay(editor)
    if (editor.showBrush) BrushDialog(editor.tool, editor.displayedBrush(), editor.brushPreview, editor::brush) { editor.brushDialog(false) }
    editor.textDraft?.let { draft -> TextEditor(draft, editor.busy || editor.pending != 0, editor::saveText, editor::dismissText) }
    if (showExport && editor.document != null) ExportDialog(editor, onDismiss = { exportDialog(false) }, onSave = {
        val ticket = editor.exportTicket()
        if (ticket == null) { if (editor.message == null) editor.transferMessage("The export settings changed. Check them again.") }
        else {
        val title = editor.info?.title?.replace(Regex("[^A-Za-z0-9 _-]"), "")?.take(80)?.ifBlank { "canvas" } ?: "canvas"
        val suffix = if (ticket.options.marked) "marked" else "clean"
        try {
            editor.instructionModal(InstructionFocusModal.ExportPicker, true)
            pickerTicketId = ticket.id
            exporter.launch(Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                .setType(ticket.encoding.mime).putExtra(Intent.EXTRA_TITLE, "$title-$suffix.${ticket.encoding.extension}"))
            exportDialog(false)
        } catch (_: Exception) { editor.cancelExportTicket(ticket.id); pickerTicketId = null; editor.instructionModal(InstructionFocusModal.ExportPicker, false); editor.transferMessage("A file destination is unavailable on this device.") }
        }
    }, onShare = { exportDialog(false); editor.sharePng() })
    cameraDiscardToken?.let { token -> AlertDialog(onDismissRequest = { cameraDiscardToken = null }, title = { Text("Discard this camera original?") },
        text = { Text("This retained capture may be its only copy. Save the original first if you want to keep it.") },
        confirmButton = { TextButton(onClick = { cameraDiscardToken = null; editor.discardCamera(token) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Discard original") } },
        dismissButton = { TextButton(onClick = { cameraDiscardToken = null }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Keep") } }) }
}

@Composable
private fun EditorScreen(editor: EditorController, onExport: () -> Unit, onSelectionSave: (SelectionSaveTicket) -> Unit) {
    val eraserState by editor.erasers.state.collectAsState()
    val selectionState by editor.selections.state.collectAsState()
    val eraserSelected = eraserState.active || (selectionState.active && selectionState.tool == com.visualworkbench.shared.SelectionTool.MaskEraser)
    Column(Modifier.fillMaxSize()) {
        Surface(elevation = 4.dp) {
            Column {
                Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    TextButton(onClick = { editor.navigate(WorkbenchScreen.Projects) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("← Projects") }
                    Text(editor.document?.title ?: "Opening…", modifier = Modifier.weight(1f).padding(horizontal = 8.dp), maxLines = 1, overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.subtitle1)
                    TextButton(onClick = editor::openAi,enabled=!editor.busy&&editor.pending==0,modifier=Modifier.heightIn(min=48.dp)){Text("AI edit")}
                    TextButton(onClick = { editor.navigate(WorkbenchScreen.Settings) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Settings") }
                    TextButton(onClick = { editor.navigate(WorkbenchScreen.Pairing) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Connect") }
                    TextButton(onClick = editor::editInstruction, modifier = Modifier.heightIn(min = 48.dp)) { Text("Instructions") }
                }
                Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 12.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(if (editor.pending > 0) "Saving ${editor.pending} edit${if (editor.pending == 1) "" else "s"}…" else editor.exportReady?.takeIf { it.matches(editor.info, editor.document) }
                            ?.let { "EXPORT READY · revision ${it.hostSeq} · ${it.width} × ${it.height} px" } ?: editor.connectionLabel(),
                            style = MaterialTheme.typography.caption, color = MaterialTheme.colors.secondary)
                        editor.document?.let { Text("${it.width} × ${it.height} px · ${it.bitDepth}-bit original", style = MaterialTheme.typography.caption) }
                    }
                    OutlinedButton(onClick = onExport, enabled = !editor.busy && editor.pending == 0 && editor.document != null,
                        modifier = Modifier.heightIn(min = 48.dp)) { Text("Export") }
                }
                if (editor.connection?.peerViewport?.documentId == editor.document?.documentId && editor.connection?.peerViewport != null) {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp)) {
                        TextButton(onClick = editor::matchPeer, modifier = Modifier.heightIn(min = 48.dp)) { Text("Match computer view") }
                        TextButton(onClick = { editor.followPeer(!editor.followPeer) }, modifier = Modifier.heightIn(min = 48.dp)) {
                            Text(if (editor.followPeer) "Stop following" else "Follow computer view")
                        }
                    }
                }
            }
        }
        AiResultStatus(editor)
        if (editor.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        editor.transferLabel?.let { label ->
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(label, Modifier.weight(1f), style = MaterialTheme.typography.body2)
                TextButton(onClick = editor::cancelTransfer, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel transfer") }
            }
        }
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
            AndroidView(factory = { context -> CanvasSurface(context, editor) }, update = { it.updatePreferences() }, modifier = Modifier.fillMaxSize())
            editor.scene?.takeIf { it.background != null && editor.blockingMessage == null }?.let { scene -> EraserOverlay(editor.erasers, scene.camera, scene.document.render.revision, scene.document.documentId) }
            editor.scene?.takeIf { it.background != null && editor.blockingMessage == null }?.let { scene -> SelectionOverlay(editor.selections, scene.camera, scene.document.render.revision, scene.document.documentId) }
            val availableHeight = maxHeight
            val railHeight = minOf(availableHeight * .78f, 560.dp).coerceAtLeast(80.dp)
            val travel = (availableHeight - railHeight - 16.dp).coerceAtLeast(0.dp)
            val density = LocalDensity.current
            ToolRail(if (eraserSelected) EditorTool.Eraser else editor.tool, editor::chooseTool, { dy ->
                val range = with(density) { travel.toPx() }
                if (range > 0) editor.settings(editor.preferences.copy(railOffset = (editor.preferences.railOffset + dy / range).coerceIn(0f, 1f)))
            }, Modifier.align(if (editor.preferences.leftHanded) Alignment.TopEnd else Alignment.TopStart)
                .padding(horizontal = 8.dp).offset(y = 8.dp + travel * editor.preferences.railOffset).height(railHeight))
            if (editor.showContext) {
                Surface(Modifier.align(Alignment.Center).padding(24.dp), shape = RoundedCornerShape(18.dp), elevation = 12.dp) {
                    Column(Modifier.padding(12.dp)) {
                        TextButton(onClick = { editor.fit(); editor.showContext = false }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Fit image") }
                        TextButton(onClick = { editor.brushDialog(true); editor.showContext = false }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Brush settings") }
                        if (editor.selected != null) {
                            TextButton(onClick = editor::deleteSelected, modifier = Modifier.heightIn(min = 48.dp)) { Text("Delete selected object") }
                            if (editor.scene?.objects?.any { it.item.objectId == editor.selected && it.item.shape is Shape.Text } == true) {
                                TextButton(onClick = { editor.editSelectedText(); editor.showContext = false }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Edit text") }
                            }
                        }
                        TextButton(onClick = { editor.showContext = false }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Close menu") }
                    }
                }
            }
            editor.blockingMessage?.let { reason ->
                Surface(Modifier.align(Alignment.Center).padding(24.dp).widthIn(max = 440.dp), shape = RoundedCornerShape(16.dp), elevation = 12.dp) {
                    Column(Modifier.padding(20.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text("Original preserved", style = MaterialTheme.typography.h6)
                        Text(reason)
                        if (editor.needsColorConsent) Button(onClick = editor::assumeSrgb, enabled = !editor.busy,
                            modifier = Modifier.heightIn(min = 48.dp)) { Text("Use sRGB for untagged colors") }
                        OutlinedButton(onClick = { editor.navigate(WorkbenchScreen.Projects) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Back to projects") }
                    }
                }
            }
            if (editor.tool == EditorTool.Select && editor.selected != null) Text("Drag to move · corner handles resize", style = MaterialTheme.typography.caption,
                color = MaterialTheme.colors.onSurface, modifier = Modifier.align(Alignment.BottomCenter).padding(8.dp).background(MaterialTheme.colors.surface, RoundedCornerShape(8.dp)).padding(8.dp))
        }
        EraserControls(editor.erasers, editor::chooseEraser, editor::stopErasing, Modifier.fillMaxWidth())
        SelectionControls(editor.selections, editor.selected, onSelectionSave, Modifier.fillMaxWidth())
        QuickControls(editor.displayedBrush(), editor::brush, { editor.brushDialog(true) },
            editor.info?.canUndo == true && editor.pending == 0, editor.info?.canRedo == true && editor.pending == 0, editor::undo, editor::redo)
    }
}

@Composable
private fun TextEditor(draft: TextDraft, saving: Boolean, save: (String, String, Double) -> Unit, dismiss: () -> Unit) {
    var text by remember(draft) { mutableStateOf(draft.text) }
    var font by remember(draft) { mutableStateOf(draft.font) }
    var size by remember(draft) { mutableStateOf(draft.size.toFloat()) }
    AlertDialog(onDismissRequest = dismiss, title = { Text(if (draft.objectId == null) "Add text" else "Edit text") }, text = {
        Column(Modifier.heightIn(max = 520.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(text, { if (it.length <= 16_384) text = it }, label = { Text("Annotation text") },
                modifier = Modifier.fillMaxWidth().heightIn(min = 112.dp).semantics { contentDescription = "Annotation text" })
            Text("Bundled font")
            Column {
                listOf("Inter", "Noto Sans", "Noto Sans Mono").forEach { candidate ->
                    TextButton(onClick = { font = candidate }, modifier = Modifier.heightIn(min = 48.dp).fillMaxWidth()) {
                        Text(if (font == candidate) "✓ $candidate" else candidate)
                    }
                }
            }
            Text("${size.toInt()} document pixels")
            Slider(size, { size = it }, valueRange = 4f..512f, modifier = Modifier.heightIn(min = 48.dp).semantics { contentDescription = "Text size" })
            Text("The canvas and exports use the same bundled font outlines.", style = MaterialTheme.typography.caption)
        }
    }, confirmButton = { TextButton(onClick = { save(text, font, size.toDouble()) }, enabled = !saving && text.isNotBlank(), modifier = Modifier.heightIn(min = 48.dp)) { Text(if (saving) "Saving…" else "Save text") } },
        dismissButton = { TextButton(onClick = dismiss, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel") } })
}
