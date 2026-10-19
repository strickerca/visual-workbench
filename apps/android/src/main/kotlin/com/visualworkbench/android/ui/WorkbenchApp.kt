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
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.visualworkbench.android.editor.*
import com.visualworkbench.android.airactions.AirActionControls
import com.visualworkbench.android.airactions.AirActionDialogControls
import com.visualworkbench.android.airactions.AirActionsPanel
import com.visualworkbench.shared.Shape
import com.visualworkbench.shared.EditableKind
import com.visualworkbench.shared.DocumentExport
import com.visualworkbench.android.precision.PrecisionControls
import com.visualworkbench.android.precision.PrecisionHandles
import com.visualworkbench.android.instructions.InstructionsScreen
import com.visualworkbench.android.instructions.InstructionFocusModal
import com.visualworkbench.shared.SelectionSaveTicket

@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
@Composable
internal fun WorkbenchApp(editor: EditorController, onCamera: () -> Unit = {}) {
    val context = LocalContext.current
    val documentState by editor.documents.state.collectAsState()
    var showExport by remember { mutableStateOf(false) }
    var showAirSettings by remember(editor) { mutableStateOf(false) }
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
        editor.interruptAirInput()
        selectionPicker = ticket
        try { selectionExporter.launch("selection-${ticket.kind.name.lowercase()}-${ticket.binding.hostSeq}.png") }
        catch (_: Exception) { selectionPicker = null; editor.transferMessage("The PNG destination picker is unavailable.") }
    }
    var pickerTicketId by rememberSaveable { mutableStateOf<String?>(null) }
    var cameraSaveToken by rememberSaveable { mutableStateOf<String?>(null) }
    var cameraDiscardToken by remember { mutableStateOf<String?>(null) }
    var documentUri by remember{mutableStateOf<android.net.Uri?>(null)}
    var documentKind by remember{mutableStateOf(EditableKind.Pdf)}
    var documentPassword by remember{mutableStateOf("")}
    var documentCopy by remember{mutableStateOf<DocumentExport?>(null)}
    var showImageSource by remember { mutableStateOf(false) }
    var largeJpegSrgb by rememberSaveable { mutableStateOf(false) }
    var largeJpegPickerOpen by rememberSaveable { mutableStateOf(false) }
    val airModalOwner = remember(editor) { Any() }
    val airModalActive = showExport || selectionPicker != null || pickerTicketId != null || cameraSaveToken != null ||
        cameraDiscardToken != null || documentUri != null || documentCopy != null || showImageSource || largeJpegPickerOpen
    DisposableEffect(editor, airModalActive) {
        editor.airModal(airModalOwner, airModalActive)
        onDispose { editor.airModal(airModalOwner, false) }
    }
    DisposableEffect(editor){onDispose{editor.instructionModal(InstructionFocusModal.DocumentImport,false);editor.instructionModal(InstructionFocusModal.DocumentPicker,false)}}
    val documentPicker=rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()){uri->
        documentUri=uri
        if(uri==null)editor.instructionModal(InstructionFocusModal.DocumentImport,false)
    }
    val documentSaver=rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()){result->
        val copy=documentCopy;documentCopy=null
        val uri=result.data?.data
        try{if(result.resultCode==Activity.RESULT_OK&&uri!=null){if(copy!=null)editor.publishDocument(uri,copy)else editor.selections.discardUnused(uri)}}
        finally{editor.instructionModal(InstructionFocusModal.DocumentPicker,false)}
    }
    fun saveDocument(copy:DocumentExport){
        if(!editor.documents.exportPublicationAvailable||documentCopy!=null)return
        documentCopy=copy;editor.instructionModal(InstructionFocusModal.DocumentPicker,true)
        try{documentSaver.launch(Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType(if(copy.mode==com.visualworkbench.shared.DocumentCopy.SvgAnnotations)"image/svg+xml" else "application/pdf").putExtra(Intent.EXTRA_TITLE,if(copy.mode==com.visualworkbench.shared.DocumentCopy.SvgAnnotations)"document.svg" else "document.pdf"))}
        catch(_:Exception){documentCopy=null;editor.instructionModal(InstructionFocusModal.DocumentPicker,false);editor.transferMessage("A document destination picker is unavailable.")}
    }
    if(documentUri!=null){
        fun dismissDocumentImport(){documentUri=null;documentPassword="";editor.instructionModal(InstructionFocusModal.DocumentImport,false)}
        AlertDialog(onDismissRequest=::dismissDocumentImport,title={Text("Open document")},text={Column{
            Row{RadioButton(documentKind==EditableKind.Pdf,{documentKind=EditableKind.Pdf});Text("PDF");RadioButton(documentKind==EditableKind.Svg,{documentKind=EditableKind.Svg});Text("SVG")}
            Text("PDF-compatible Illustrator (.ai) files use PDF admission. Files without a compatible PDF stream are refused; no flattened substitute is created.")
            if(documentKind==EditableKind.Pdf)OutlinedTextField(documentPassword,{documentPassword=it.take(1024)},label={Text("PDF password (if required)")},visualTransformation=androidx.compose.ui.text.input.PasswordVisualTransformation())
        }},confirmButton={TextButton({val uri=documentUri;val password=documentPassword.takeIf{it.isNotEmpty()};val kind=documentKind;dismissDocumentImport();if(uri!=null)editor.importDocument(uri,kind,if(kind==EditableKind.Pdf)password else null)}){Text("Open")}},dismissButton={TextButton(::dismissDocumentImport){Text("Cancel")}})
    }
    val imagePicker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uri?.let { editor.receiveImage(it) } }
    val largeJpegPicker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        val admitted = largeJpegPickerOpen && largeJpegSrgb
        largeJpegPickerOpen = false; largeJpegSrgb = false
        if (uri != null && admitted) editor.receiveGiantImage(uri, assumeUntaggedSrgb = true)
    }
    val photoPicker = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri -> uri?.let { editor.receiveImage(it) } }
    if (showImageSource) AlertDialog(onDismissRequest = { showImageSource = false; largeJpegSrgb = false },
        title = { Text("Open image file") }, text = { Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Choose regular import for PNG, JPEG, WebP, HEIF, AVIF, BMP or TIFF within the current decoder limits.")
            Text("Large JPEG uses original-resolution tiles. The selected file must actually be JPEG; its original is saved unchanged.")
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(largeJpegSrgb, { largeJpegSrgb = it })
                Text("For a large JPEG without a color profile, treat colors as sRGB for tiled display.")
            }
        } }, confirmButton = {
            TextButton(onClick = {
                showImageSource = false
                largeJpegPickerOpen = true
                try { largeJpegPicker.launch(arrayOf("image/jpeg")) }
                catch (_: Exception) { largeJpegPickerOpen = false; largeJpegSrgb = false; editor.transferMessage("A large-JPEG document picker is unavailable.") }
            }, enabled = largeJpegSrgb) { Text("Large JPEG") }
        }, dismissButton = {
            TextButton(onClick = {
                showImageSource = false; largeJpegSrgb = false
                try { imagePicker.launch(arrayOf("image/png", "image/jpeg", "image/webp", "image/heic", "image/heif", "image/avif", "image/bmp", "image/tiff")) }
                catch (_: Exception) { editor.transferMessage("An image document picker is unavailable.") }
            }) { Text("Regular image") }
        })
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
                    { showImageSource = true }, onCamera,
                    editor.cameraCaptures, editor::retryCamera, { cameraDiscardToken = it }, { token ->
                        if (cameraSaveToken != null) editor.transferMessage("Finish or cancel the pending original-file destination first.")
                        else {
                            cameraSaveToken = token
                            try { cameraSaver.launch("camera-original") }
                            catch (_: Exception) { cameraSaveToken = null; editor.transferMessage("A file destination is unavailable on this device.") }
                        }
                    },onDocument={
                        editor.cancelTool();editor.instructionModal(InstructionFocusModal.DocumentImport,true)
                        try{documentPicker.launch(arrayOf("application/pdf","image/svg+xml","application/postscript","application/illustrator","application/octet-stream"))}catch(_:Exception){editor.instructionModal(InstructionFocusModal.DocumentImport,false);editor.transferMessage("A document picker is unavailable.")}
                    })
                WorkbenchScreen.Canvas -> EditorScreen(editor, { exportDialog(true) }, ::saveSelection,::saveDocument)
                WorkbenchScreen.Settings -> Column {
                    TextButton(onClick = { context.startActivity(Intent(context, com.visualworkbench.android.capture.CaptureEntryActivity::class.java)) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Screen capture and accessibility") }
                    TextButton(onClick = { showAirSettings = !showAirSettings }, modifier = Modifier.heightIn(min = 48.dp)) {
                        Text(if (showAirSettings) "Back to editor settings" else "Air Actions setup and remapping")
                    }
                    if (showAirSettings) {
                        TextButton(onClick = editor::returnFromSettings, modifier = Modifier.heightIn(min = 48.dp)) { Text("Back to document") }
                        AndroidView(factory = { AirActionsPanel(it) }, update = { panel ->
                            panel.render(editor.airSetup, editor.preferences.airActions, editor::airCommandEnabled,
                                editor::configureAirSetup, editor::remapAirAction, editor::airControl)
                        }, modifier = Modifier.weight(1f).fillMaxWidth())
                    } else Box(Modifier.weight(1f)) { SettingsScreen(editor.preferences, editor::settings, editor::returnFromSettings) }
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
    if(editor.remoteOpen)editor.remoteCapability?.let { remote ->
        com.visualworkbench.android.remote.RemoteEditPanel(remote, onDismiss = editor::remoteClosed,
            airActions = { palette, blocked -> AirActionDialogControls(editor, remote, palette, blocked) })
    }
    AiEditorOverlay(editor)
    if (editor.showBrush) BrushDialog(editor.tool, editor.displayedBrush(), editor.brushPreview, editor::brush) { editor.brushDialog(false) }
    editor.textDraft?.let { draft -> TextEditor(draft, editor.busy || editor.pending != 0, editor::saveText, editor::dismissText) }
    if(showExport&&documentState.descriptor!=null)AlertDialog(onDismissRequest={exportDialog(false)},title={Text("Document export")},text={Column(Modifier.heightIn(max=560.dp).verticalScroll(rememberScrollState())){EditableDocumentControls(editor.documents,::saveDocument)}},confirmButton={TextButton({exportDialog(false)}){Text("Close")}})
    if (showExport && editor.document != null && documentState.descriptor==null) ExportDialog(editor, onDismiss = { exportDialog(false) }, onSave = {
        val ticket = editor.exportTicket()
        if (ticket == null) { if (editor.message == null) editor.transferMessage("The export settings changed. Check them again.") }
        else {
        val title = editor.info?.title?.replace(Regex("[^A-Za-z0-9 _-]"), "")?.take(80)?.ifBlank { "canvas" } ?: "canvas"
        val suffix = if (ticket.options.marked) "marked" else "clean"
        try {
            editor.instructionModal(InstructionFocusModal.ExportPicker, true)
            pickerTicketId = ticket.id
            exporter.launch(Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                .setType(ticket.encoding.exportMimeType()).putExtra(Intent.EXTRA_TITLE, "$title-$suffix.${ticket.encoding.exportFileSuffix()}"))
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
private fun EditorScreen(editor: EditorController, onExport: () -> Unit, onSelectionSave: (SelectionSaveTicket) -> Unit,onDocumentSave:(DocumentExport)->Unit) {
    val documentState by editor.documents.state.collectAsState()
    val eraserState by editor.erasers.state.collectAsState()
    val selectionState by editor.selections.state.collectAsState()
    val eraserSelected = eraserState.active || (selectionState.active && selectionState.tool == com.visualworkbench.shared.SelectionTool.MaskEraser)
    var projectActionsOpen by remember(editor.document?.documentId) { mutableStateOf(false) }
    var pixelSelectionOpen by remember(editor.document?.documentId) { mutableStateOf(false) }
    val airModalOwner = remember(editor) { Any() }
    DisposableEffect(editor, projectActionsOpen, pixelSelectionOpen) {
        editor.airModal(airModalOwner, projectActionsOpen || pixelSelectionOpen)
        onDispose { editor.airModal(airModalOwner, false) }
    }
    BoxWithConstraints(Modifier.fillMaxSize()) {
    val contextHeight = (maxHeight * .24f).coerceIn(48.dp, 144.dp)
    Column(Modifier.fillMaxSize()) {
        Surface(elevation = 4.dp) {
            Column {
                Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    TextButton(onClick = { editor.navigate(WorkbenchScreen.Projects) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("← Projects") }
                    Text(editor.document?.title ?: "Opening…", modifier = Modifier.weight(1f).padding(horizontal = 8.dp), maxLines = 1, overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.subtitle1)
                    Box {
                        TextButton(onClick = { editor.interruptAirInput(); projectActionsOpen = true }, modifier = Modifier.heightIn(min = 48.dp)
                            .semantics { contentDescription = "More project actions" }) { Text("More") }
                        DropdownMenu(projectActionsOpen, { projectActionsOpen = false }) {
                            DropdownMenuItem(onClick = { projectActionsOpen = false; editor.openAi() }, enabled = !editor.busy && editor.pending == 0) { Text("AI edit") }
                            DropdownMenuItem(onClick = { projectActionsOpen = false; editor.navigate(WorkbenchScreen.Settings) }) { Text("Settings") }
                            DropdownMenuItem(onClick = { projectActionsOpen = false; editor.navigate(WorkbenchScreen.Pairing) }) { Text("Connect") }
                            if (editor.connection?.peerViewport?.documentId == editor.document?.documentId && editor.connection?.peerViewport != null) {
                                DropdownMenuItem(onClick = { projectActionsOpen = false; editor.matchPeer() }) { Text("Match computer view") }
                                DropdownMenuItem(onClick = { projectActionsOpen = false; editor.followPeer(!editor.followPeer) }) {
                                    Text(if (editor.followPeer) "Stop following" else "Follow computer view")
                                }
                            }
                            DropdownMenuItem(onClick = { projectActionsOpen = false; editor.editInstruction() }) { Text("Instructions") }
                            DropdownMenuItem(onClick = { projectActionsOpen = false; pixelSelectionOpen = true }, enabled = !editor.busy && editor.document != null && documentState.descriptor==null) { Text("Pixel selection") }
                            DropdownMenuItem(onClick = { projectActionsOpen = false; onExport() }, enabled = !editor.busy && editor.pending == 0 && editor.document != null) { Text("Export") }
                        }
                    }
                }
                Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 12.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(if (editor.pending > 0) "Saving ${editor.pending} edit${if (editor.pending == 1) "" else "s"}…" else editor.exportReady?.takeIf { it.matches(editor.info, editor.document) }
                            ?.let { "EXPORT READY · revision ${it.hostSeq} · ${it.width} × ${it.height} px" } ?: editor.connectionLabel(),
                            style = MaterialTheme.typography.caption, color = MaterialTheme.colors.secondary)
                        editor.document?.let { Text(if(documentState.descriptor==null)"${it.width} × ${it.height} px · ${it.bitDepth}-bit original" else "${it.width} × ${it.height} document units", style = MaterialTheme.typography.caption) }
                    }
                }
            }
        }
        PrecisionControls(editor.precision)
        AirActionControls(editor)
        EditorWorkflowControls(editor)
        if(documentState.descriptor!=null)Column(Modifier.fillMaxWidth().heightIn(max=220.dp).verticalScroll(rememberScrollState())){EditableDocumentControls(editor.documents,onDocumentSave)}
        AiResultStatus(editor)
        if (editor.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        editor.transferLabel?.let { label ->
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(label, Modifier.weight(1f), style = MaterialTheme.typography.body2)
                TextButton(onClick = editor::cancelTransfer, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel transfer") }
            }
        }
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
            AndroidView(factory = { context -> CanvasSurface(context, editor) }, update = { it.updatePreferences() }, modifier = Modifier.fillMaxSize().clipToBounds())
            editor.scene?.takeIf { (it.background != null||it.documentFrame!=null||it.giantFrame!=null) && editor.blockingMessage == null }?.let { scene -> EraserOverlay(editor.erasers, scene.camera, scene.document.render.revision, scene.document.documentId) }
            editor.scene?.takeIf { (it.background != null||it.documentFrame!=null||it.giantFrame!=null) && editor.blockingMessage == null }?.let { scene -> SelectionOverlay(editor.selections, scene.camera, scene.document.render.revision, scene.document.documentId) }
            PrecisionHandles(editor.precision)
            if(editor.giantMode&&editor.scene?.giantFrame==null&&editor.blockingMessage==null){
                Column(Modifier.align(Alignment.Center).padding(24.dp),verticalArrangement=Arrangement.spacedBy(8.dp)){
                    Text(editor.giantIssue?:"Preparing original-image tiles…")
                    if(editor.giantIssue!=null)TextButton(editor::retryGiantDisplay,Modifier.heightIn(min=48.dp),enabled=!editor.busy){Text("Retry image display")}
                }
            }
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
            if (editor.tool == EditorTool.Select && editor.selected != null) Text("Drag to move · corner handles resize", style = MaterialTheme.typography.caption,
                color = MaterialTheme.colors.onSurface, modifier = Modifier.align(Alignment.BottomCenter).padding(8.dp).background(MaterialTheme.colors.surface, RoundedCornerShape(8.dp)).padding(8.dp))
        }
        val eraserControlsVisible = eraserState.busy || (eraserState.active && !selectionState.active)
        val selectionControlsVisible = selectionState.active || selectionState.busy
        val inactiveEraseMessage = eraserState.message?.takeIf { !eraserControlsVisible }
        if (eraserControlsVisible || selectionControlsVisible || inactiveEraseMessage != null) {
            Column(Modifier.fillMaxWidth().heightIn(max = contextHeight).verticalScroll(rememberScrollState())) {
                if (eraserControlsVisible) EraserControls(editor.erasers, editor::chooseEraser, editor::stopErasing, Modifier.fillMaxWidth())
                if (selectionControlsVisible) SelectionControls(editor.selections, editor.selected, onSelectionSave, Modifier.fillMaxWidth(),
                    onOpen = { editor.interruptAirInput(); pixelSelectionOpen = true })
                inactiveEraseMessage?.let { status ->
                    Row(Modifier.fillMaxWidth().padding(start = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(status, Modifier.weight(1f), style = MaterialTheme.typography.caption)
                        TextButton(onClick = editor.erasers.interaction::dismissMessage, modifier = Modifier.heightIn(min = 48.dp)
                            .semantics { contentDescription = "Dismiss erase status" }) { Text("Dismiss") }
                    }
                }
            }
        }
        editor.originalFormatNotice?.let { Text(it, Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), style = MaterialTheme.typography.caption) }
        QuickControls(editor.displayedBrush(), editor::brush, { editor.brushDialog(true) },
            editor.info?.canUndo == true && editor.pending == 0, editor.info?.canRedo == true && editor.pending == 0, editor::undo, editor::redo)
    }
    if (pixelSelectionOpen && editor.blockingMessage == null) SelectionPanel(editor.selections, editor.selected,
        { ticket -> pixelSelectionOpen = false; onSelectionSave(ticket) }, { pixelSelectionOpen = false })
    editor.blockingMessage?.let { reason ->
        AlertDialog(onDismissRequest = { editor.navigate(WorkbenchScreen.Projects) }, title = { Text("Original preserved") },
            text = {
                Column(Modifier.heightIn(max = 320.dp).verticalScroll(rememberScrollState())) {
                    editor.document?.title?.let { Text(it, style = MaterialTheme.typography.subtitle1) }
                    Text(reason)
                }
            },
            confirmButton = {
                if (editor.needsColorConsent) Button(onClick = editor::assumeSrgb, enabled = !editor.busy,
                    modifier = Modifier.heightIn(min = 48.dp)) { Text("Use sRGB for untagged colors") }
            }, dismissButton = {
                TextButton(onClick = { editor.navigate(WorkbenchScreen.Projects) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Back to projects") }
            })
    }
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
