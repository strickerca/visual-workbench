@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class,kotlinx.coroutines.FlowPreview::class)
package com.visualworkbench.desktop

import com.visualworkbench.desktop.instructions.InstructionsPanel

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import com.visualworkbench.bindings.host.*
import com.visualworkbench.shared.*
import com.visualworkbench.desktop.mcp.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.debounce
import java.awt.FileDialog
import java.awt.GraphicsEnvironment
import java.nio.file.Path
import javax.swing.filechooser.FileSystemView

private val accent=Color(0xff81e0c8)
private val palette=listOf(0x78dcc8ffu,0xffd580ffu,0xf68282ffu,0x91b7ffffu,0xffffffffu,0x182535ffu)
fun main(args: Array<String>) {
    // Refuse unknown/duplicate arguments before native, preference or UI work.
    require(args.isEmpty() || args.contentEquals(arrayOf("--startup-smoke")) || args.contentEquals(arrayOf("--mcp-runtime-smoke")) || args.contentEquals(arrayOf("--mcp-minimized"))) { "Unsupported desktop argument" }
    val mcpRuntimeSmoke = args.contentEquals(arrayOf("--mcp-runtime-smoke"))
    val startupSmoke = args.contentEquals(arrayOf("--startup-smoke")) || mcpRuntimeSmoke
    val mcpMinimized = args.contentEquals(arrayOf("--mcp-minimized"))
    val started=System.nanoTime()
    val local=Path.of(System.getenv("LOCALAPPDATA")?:error("LOCALAPPDATA unavailable"))
    val nativeRuntime=DesktopNativeRuntime.prepare(local)
    nativeRuntime.activate()
    Runtime.getRuntime().addShutdownHook(Thread({nativeRuntime.close()},"vw-native-release"))
    // Establish awareness before the first AWT/Compose window.
    val dpi=setProcessPerMonitorV2();check(dpi.perMonitorV2){"Per-monitor V2 DPI awareness is required"}
    val appAnchor=NativeCacheAnchor.prepare(local,WindowsNativeFileGuard())
    val appOwner=DesktopAppOwner.claim(appAnchor.directory)
    if(appOwner==null){
        try {
            DesktopAppOwner.request(appAnchor.directory,if(mcpMinimized)AppStartRequest.Mcp else AppStartRequest.Show)
            if(!mcpMinimized)javax.swing.JOptionPane.showMessageDialog(null,"Visual Workbench is already running. Its existing window was requested. Use its taskbar button if it stays minimized.","Visual Workbench",javax.swing.JOptionPane.INFORMATION_MESSAGE)
        } catch(_:Exception){javax.swing.JOptionPane.showMessageDialog(null,"Visual Workbench is already running, but the handoff did not complete. Open its taskbar window; no second project owner was started.","Visual Workbench",javax.swing.JOptionPane.WARNING_MESSAGE)}
        finally{appAnchor.close();nativeRuntime.close()}
        return
    }
    Runtime.getRuntime().addShutdownHook(Thread({appOwner.close();appAnchor.close()},"vw-app-owner-release"))
    val core=workbenchCore()
    val preferences=DesktopPreferences(local.resolve("Visual Workbench/settings.properties"));val saved=preferences.window();val device=preferences.device(core)
    val documents=FileSystemView.getFileSystemView().defaultDirectory.toPath()
    val clouds=listOf("OneDrive","OneDriveConsumer","OneDriveCommercial","DROPBOX").mapNotNull{System.getenv(it)?.takeIf(String::isNotBlank)?.let{v->Path.of(v)}}
    val location=ProjectLocations.choose(documents,local,clouds)
    val host=runBlocking(Dispatchers.Default){createHostService()}
    val captureService=runBlocking(Dispatchers.Default){com.visualworkbench.desktop.capture.openDesktopCaptureService(
        nativeRuntime.directory.resolve("vw-capture-helper.exe"),nativeRuntime.packagedHash("vw-capture-helper.exe"))}
    application {
        val scope=rememberCoroutineScope();val sessions=remember{DesktopSessionController(scope,device)};val assistance=remember{ConnectionAssistanceController(scope,NativeConnectionAssistance())};val controller=remember{EditorController(scope,core,host,location,device,local.resolve("VisualWorkbench/transfer-work")).also{it.useSessionService(sessions);it.useAiDirectory(local.resolve("Visual Workbench"))}};val editor by controller.state.collectAsState()
        val capture=remember{com.visualworkbench.desktop.capture.DesktopCaptureController(scope,core,captureService,location.path,device,
            controller::captureDestination,controller::report)}
        var captureSettings by remember{mutableStateOf(false)}
        var captureFocusLease by remember { mutableStateOf<AutoCloseable?>(null) }
        fun captureDialog(value: Boolean) {
            if (value && !captureSettings) captureFocusLease = controller.instructionPicker()
            captureSettings = value
            if (!value) { val prior = captureFocusLease; captureFocusLease = null; prior?.close() }
        }
        DisposableEffect(controller) { onDispose { val prior = captureFocusLease; captureFocusLease = null; prior?.close() } }
        var captureProtected by remember{mutableStateOf(false)}
        var captureChecked by remember{mutableStateOf(false)}
        val mcp=remember{DesktopMcpLifetime(appAnchor.directory,nativeRuntime.directory.resolve("vw-capture-helper.exe"),nativeRuntime.packagedHash("vw-capture-helper.exe"),core,device)}
        val codex=remember{DesktopCodexHandoff(appAnchor.directory)}
        val codexModal=remember{McpModalOwner<Unit>{controller.instructionPicker()}}
        val codexPanels by codexModal.state.collectAsState()
        DisposableEffect(codexModal){onDispose{codexModal.close()}}
        val mcpTasks=remember{McpUiTasks()};val mcpState by mcp.state.collectAsState()
        LaunchedEffect(mcp,controller) {
            // Read the current lifetime every iteration, including replacement,
            // reconnect and shutdown. Never retain a prior agent's grant list.
            while(isActive){controller.publishAgentCaptureStatus(mcpGrantTelemetry(mcp.state.value,System.nanoTime()));delay(500)}
        }
        val mcpModal=remember{McpModalOwner<McpInboxReceipt>{controller.instructionPicker()}}
        val mcpPanels by mcpModal.state.collectAsState();val mcpSettings=mcpPanels.settings;val mcpCompare=mcpPanels.compare
        DisposableEffect(mcpModal){onDispose{mcpModal.close()}}
        val state=rememberWindowState(isMinimized=mcpMinimized,placement=saved.mode.placement(),position=restoredPosition(saved),width=saved.width.dp,height=saved.height.dp)
        var prior by remember{mutableStateOf(if(saved.mode==WindowMode.Fullscreen)WindowMode.Windowed else saved.mode)}
        var floating by remember{mutableStateOf(saved.copy(mode=WindowMode.Windowed))};var dialog by remember{mutableStateOf<Command?>(null)}
        var quitting by remember{mutableStateOf(false)};var dpiText by remember{mutableStateOf("Checking display scale…")};var startupMs by remember{mutableStateOf<Long?>(null)}
        fun instructionDialog(value: Command?) { controller.instructionModal(value != null); dialog = value }
        fun quit() {
            if(!controller.mayCloseAi())return
            if (quitting || !controller.sealInstructionsForClose()) return
            if (!quitting) {
                quitting = true
                scope.launch { withContext(NonCancellable) {
                    var clean = false
                    try {
                        try { codex.close() } finally { codexModal.close()
                        try { mcpTasks.close() } finally { try { mcp.close() } finally {
                        mcpModal.close()
                        try { try { capture.shutdown() } finally { assistance.close() } } finally {
                            try { controller.close() } finally {
                                try { sessions.close() } finally {
                                    try { host.shutdown() } finally { host.destroy() }
                                }
                            }
                        }
                        } }
                        }
                        appOwner.close();appAnchor.close()
                        clean = true
                    } finally {
                        if (startupSmoke) println(if (clean)
                            "VW_DESKTOP_EXIT startup_smoke=true cleanup=complete"
                            else "VW_DESKTOP_EXIT startup_smoke=true cleanup=failed")
                        exitApplication()
                    }
                } }
            }
        }
        LaunchedEffect(appOwner){
            suspend fun startMcp(){
                try{mcpTasks.run{mcp.start()}}catch(error:CancellationException){throw error}catch(_:Exception){controller.report("The local service could not start. Existing project owners were preserved.")}
                if(mcp.state.value.issue!=null){state.isMinimized=false;mcpModal.settings()}
            }
            if(mcpMinimized)startMcp()
            while(isActive&&!quitting){
                val request=try{withContext(Dispatchers.IO){appOwner.poll()}}catch(_:Exception){controller.report("The app handoff marker was refused; existing files were preserved.");null}
                if(!quitting)when(request){AppStartRequest.Show->{state.isMinimized=false};AppStartRequest.Mcp->startMcp();null->Unit}
                delay(250)
            }
        }
        mcpState.owner?.let{McpCaptureIndicator(it)}
        fun mode(next:WindowMode){if(next==WindowMode.Fullscreen){if(state.placement!=WindowPlacement.Fullscreen){prior=state.placement.mode();state.placement=WindowPlacement.Fullscreen}else state.placement=prior.placement()}else state.placement=next.placement()}
        Window(visible=captureChecked,onCloseRequest={quit()},state=state,title=editor.document?.title?.let{"$it · Visual Workbench"}?:"Visual Workbench",undecorated=state.placement==WindowPlacement.Fullscreen,
            onPreviewKeyEvent={event->shortcutForFocus(event,editor.canvasFocused,codexPanels.settings||mcpSettings||mcpCompare!=null||captureSettings||dialog!=null||editor.textAnchor!=null||editor.exportOpen||editor.pasteOpen||controller.ai.state.value.open||(!editor.canvasFocused&&controller.instructionEditor.state.value.field!=null))?.let{dispatch(it,event.isShiftPressed,controller,{mode(it)},{instructionDialog(it)},{quit()});true}?:false}) {
            fun file(command:Command){controller.instructionPicker().use { val epoch=controller.state.value.projectEpoch;val chooser=FileDialog(window,command.label,FileDialog.LOAD)
                try{chooser.directory=(if(command==Command.Import)documents else location.path).toString();if(command==Command.Open)chooser.file="project.sqlite";chooser.isVisible=true
                    val filename=chooser.file?:return;val chosen=Path.of(chooser.directory,filename)
                    when(command){Command.Import->controller.importFile(ImportRequest(epoch,chosen.toAbsolutePath().normalize()));Command.Open->controller.open(chosen.parent,epoch);else->Unit}
                }finally{chooser.dispose()}}}
            fun save(request:ExportRequest){controller.instructionPicker().use { val chooser=FileDialog(window,"Save ${request.settings.encoding.label} · ${request.revision}",FileDialog.SAVE)
                try{chooser.directory=documents.toString();chooser.file="export-${request.revision}.${request.settings.encoding.extension}";chooser.isVisible=true;val filename=chooser.file?:return
                    val extension=filename.substringAfterLast('.',"").lowercase();val allowed=if(request.settings.encoding==ExportEncoding.Jpeg)setOf("jpg","jpeg")else setOf(request.settings.encoding.extension)
                    if(extension !in allowed){controller.report("Use a .${request.settings.encoding.extension} filename for the selected format.");return}
                    controller.saveExport(request,Path.of(chooser.directory,filename))
                }finally{chooser.dispose()}}}
            fun saveSelection(ticket:SelectionSaveTicket){controller.instructionPicker().use {
                val chooser=FileDialog(window,"Save selection PNG",FileDialog.SAVE)
                try{chooser.directory=documents.toString();chooser.file="selection-${ticket.kind.name.lowercase()}-${ticket.binding.hostSeq}.png";chooser.isVisible=true
                    val name=chooser.file?:return
                    if(!controller.selections.save(ticket,Path.of(chooser.directory,name).toAbsolutePath().normalize()))controller.report("The selection changed or another save is active. Choose Export again.")
                }finally{chooser.dispose()}
            }}
            fun chooseCodexExecutable():Path?{
                val lease=codexModal.retain()
                val chooser=try{FileDialog(window,"Select installed codex.exe",FileDialog.LOAD)}catch(error:Throwable){lease.close();throw error}
                return try{chooser.file="codex.exe";chooser.isVisible=true;val filename=chooser.file?:return null
                    Path.of(chooser.directory,filename).toAbsolutePath().normalize()
                }finally{try{chooser.dispose()}finally{lease.close()}}
            }
            fun saveMcp(receipt:McpInboxReceipt,side:McpInboxSide){
                val owner=mcpState.owner?:return
                val chooserLease=mcpModal.retain()
                val chooser=try{FileDialog(window,"Save exact ${side.name} PNG",FileDialog.SAVE)}catch(error:Throwable){chooserLease.close();throw error}
                try{chooser.directory=documents.toString();chooser.file="${side.name.lowercase()}-${receipt.receiptId}.png";chooser.isVisible=true
                    val name=chooser.file?:return;val destination=Path.of(chooser.directory,name).toAbsolutePath().normalize()
                    scope.launch{try{mcpTasks.run{exportMcpOriginal(owner,receipt,side,destination,appAnchor.directory.resolve("mcp-export-work"))}}
                        catch(error:CancellationException){throw error}catch(_:Exception){controller.report("The exact PNG export was refused or cleanup is uncertain. Existing destination files were preserved.")}}
                }finally{try{chooser.dispose()}finally{chooserLease.close()}}
            }
            fun action(command:Command){if(command in listOf(Command.Import,Command.Open))file(command)else dispatch(command,false,controller,{mode(it)},{instructionDialog(it)},{quit()})}
            LaunchedEffect(editor.textAnchor != null, editor.exportOpen, editor.pasteOpen) { controller.instructionFocus.interactionChanged() }
            LaunchedEffect(dialog){val pending=dialog;if(pending in listOf(Command.Import,Command.Open)){instructionDialog(null);file(checkNotNull(pending))}}
            DisposableEffect(window,controller){val drop=DesktopDropTarget(window.contentPane,controller);val focus=object:java.awt.event.WindowAdapter(){override fun windowDeactivated(event:java.awt.event.WindowEvent){controller.cancelInput()}};window.addWindowListener(focus);onDispose{window.removeWindowListener(focus);controller.cancelInput();drop.close()}}
            val density=LocalDensity.current.density
            LaunchedEffect(density,state.position){try{val info=host.windowDpiInfo(window.windowHandle.toULong());dpiText="Windows ${info.windowDpi} DPI · Compose ${"%.2f".format(density)}× · PMv2 ${info.windowPerMonitorV2}"}catch(_:Exception){dpiText="Display DPI query unavailable"}}
            val firstDraw = remember { CompletableDeferred<Unit>() }
            LaunchedEffect(Unit) {
                try { excludeCaptureWindows(listOf(window.windowHandle.toULong())); captureProtected=true }
                catch (_: Exception) { controller.report("Screen capture is unavailable because this editor window could not be excluded. Editing remains available.") }
                finally { captureChecked=true }
                if (startupSmoke) {
                    // The actual EditorShell draw must return; the next frame
                    // callback moves the readiness effect out of that draw.
                    firstDraw.await()
                    withFrameNanos { }
                } else withFrameNanos { }
                startupMs = (System.nanoTime() - started) / 1_000_000
                if (startupSmoke) {
                    val info = host.windowDpiInfo(window.windowHandle.toULong())
                    check(info.windowPerMonitorV2 && info.windowDpi > 0u) { "Window DPI readiness failed" }
                    println("VW_DESKTOP_READY startup_ms=$startupMs composeDensity=$density pmv2=${dpi.perMonitorV2}")
                    println("VW_DESKTOP_SMOKE_FRAME window_dpi=${info.windowDpi} window_pmv2=${info.windowPerMonitorV2} drawn=true")
                    try {
                        if (mcpRuntimeSmoke) {
                            withTimeout(90_000) { verifyPackagedMcpLifecycle(mcp, appAnchor.directory) }
                            println("VW_DESKTOP_MCP cycles=2 duplicate_owner=refused grants=0 sends=0 cleanup=complete")
                        }
                    } catch (error: Exception) {
                        println("VW_DESKTOP_MCP failed=true stage=${(error as? McpRefused)?.stage?.name ?: "unknown"}")
                    } finally { quit() }
                } else println("VW_DESKTOP_READY startup_ms=$startupMs composeDensity=$density pmv2=${dpi.perMonitorV2}")
            }
            LaunchedEffect(state){snapshotFlow{Triple(state.placement,state.size,state.position)}.debounce(350).collect{(placement,size,position)->
                if(placement==WindowPlacement.Floating){val p=position as? WindowPosition.Absolute;floating=SavedWindow(WindowMode.Windowed,size.width.value,size.height.value,p?.x?.value,p?.y?.value)}
                val snapshot=floating.copy(mode=placement.mode());withContext(Dispatchers.IO){preferences.saveWindow(snapshot)}
            }}
            MenuBar {
                Menu("File"){for(c in listOf(Command.Import,Command.Paste,Command.Open,Command.Export,Command.Copy,Command.Close,Command.Quit))Item("${c.label}\t${c.shortcut}",onClick={action(c)})}
                Menu("Edit"){for(c in listOf(Command.Undo,Command.Redo,Command.Delete,Command.SelectAll,Command.ClearSelection,Command.Color,Command.WidthUp,Command.WidthDown,Command.NudgeLeft,Command.NudgeRight,Command.NudgeUp,Command.NudgeDown))Item("${c.label}\t${c.shortcut}",onClick={action(c)})}
                Menu("Tools"){for(c in listOf(Command.Select,Command.Pan,Command.Pen,Command.Rectangle,Command.Ellipse,Command.Line,Command.Arrow,Command.Text,Command.Callout,Command.CycleMarker,Command.Instructions))Item("${c.label}\t${c.shortcut}",onClick={action(c)})}
                Menu("View"){for(c in listOf(Command.Fit,Command.ActualPixels,Command.ZoomIn,Command.ZoomOut,Command.Windowed,Command.Maximized,Command.Fullscreen,Command.FollowPeer,Command.MatchPeer,Command.PeerOutline))Item("${c.label}\t${c.shortcut}",onClick={action(c)})}
                Menu("Workbench"){Item("Send package to Codex",onClick={controller.cancelInput();codexModal.settings();mcpModal.dismiss()});Item("Local agents and Compare",onClick={controller.cancelInput();mcpModal.settings();codexModal.dismiss()});for(c in listOf(Command.Pairing,Command.Settings,Command.Diagnostics))Item("${c.label}\t${c.shortcut}",onClick={action(c)});Item("Foreground capture…",enabled=captureProtected,onClick={captureDialog(true)})}
            }
            MaterialTheme(colors=darkColors(primary=accent,secondary=Color(0xffe8bb75),background=Color(0xff101720),surface=Color(0xff18222e))){
                Box(Modifier.fillMaxSize().drawWithContent {
                    drawContent()
                    if (startupSmoke) firstDraw.complete(Unit)
                }) { EditorShell(controller,editor,dpiText,quitting,::action,::saveSelection) }
                if(codexPanels.settings){
                    val catalogOwner=mcpState.owner
                    val published=if(catalogOwner!=null){val catalog by catalogOwner.state.collectAsState();catalog.packages}else emptyList()
                    CodexPanel(codex,published,::chooseCodexExecutable){codexModal.dismiss()}
                }
                if(mcpSettings){
                    val owner=mcpState.owner
                    if(owner!=null)McpSettingsPanel(owner,{choice->mcpTasks.run{
                        val value=controller.compileForMcp(choice);try{owner.publish(value)}finally{withContext(NonCancellable){value.close()}}
                    }},{receipt->mcpModal.compare(receipt)},{mcpModal.dismiss()},onStop={
                        scope.launch(start=CoroutineStart.UNDISPATCHED){try{mcpModal.settings();mcp.stop()}catch(_:Exception){controller.report("Local service shutdown is uncertain. Restart the application before admitting another owner.")}}
                    })
                    else AlertDialog(onDismissRequest={mcpModal.dismiss()},title={Text("Local agent access")},text={Text(mcpState.issue?:"Start the local service to expose explicitly published packages. Capture grants and Send stay off until your separate actions.")},
                        confirmButton={Button(enabled=!mcpState.starting&&!mcpState.closed,onClick={scope.launch{try{mcpTasks.run{mcp.start()}}catch(error:CancellationException){throw error}catch(_:Exception){controller.report("The local service is busy or unavailable.")}}}){Text(if(mcpState.starting)"Starting…" else "Start local service")}},
                        dismissButton={TextButton(onClick={mcpModal.dismiss()}){Text("Close")}})
                }
                mcpCompare?.let{receipt->mcpState.owner?.let{owner->McpComparePanel(owner,receipt,{mcpModal.dismiss()},::saveMcp)}}
                AiEditorOverlay(controller)
                if(captureSettings)com.visualworkbench.desktop.capture.CaptureSettings(capture){captureDialog(false)}
                if(editor.exportOpen&&editor.document!=null)ExportDialog(controller,editor,::save)
                if(editor.pasteOpen)PasteDialog(controller,editor)
                if(dialog==Command.Pairing)SessionDialog(sessions,controller,assistance){instructionDialog(null)}
                if(dialog in listOf(Command.Settings,Command.Diagnostics)){
                    val command=checkNotNull(dialog);val timing=editor.sync?.let{"${it.echoSamples} echoes · RTT p50 ${it.echoRttP50Ms?:0.0} ms · p95 ${it.echoRttP95Ms?:0.0} ms · clock offset ${it.clockOffsetMs?:0.0} ms"}?:"No echo samples"
                    val text=when(command){Command.Settings->"${location.reason}\n${location.path}\n\nDisplay budget: 256 MiB, full resolution. Original image bytes are preserved. Finger and trackpad gestures navigate the local camera.";Command.Diagnostics->"$dpiText\nFirst frame: ${startupMs?:0} ms\nCore binding: 1\n${syncLabel(editor.sync)}\n$timing\nEcho RTT is not end-to-end drawing latency.\nDocument rendering and edits use the native core.";else->"Open the instruction panel to edit saved marker and object instructions."}
                    AlertDialog(onDismissRequest={instructionDialog(null)},title={Text(command.label)},text={Text(text)},confirmButton={TextButton(onClick={instructionDialog(null)}){Text("Close")}})
                }
                editor.textAnchor?.let{var text by remember{mutableStateOf("")};AlertDialog(onDismissRequest={if(!editor.busy)controller.clearSelection()},title={Text("Text annotation")},text={Column{OutlinedTextField(text,{text=it.take(16384)},enabled=!editor.busy,label={Text("Text · Inter 18 px")});editor.message?.let{Text(it,style=MaterialTheme.typography.caption)}}},confirmButton={Button(onClick={controller.acceptText(text)},enabled=text.isNotBlank()&&!editor.busy){Text("Add text")}},dismissButton={TextButton(onClick={controller.clearSelection()},enabled=!editor.busy){Text("Cancel")}})}
            }
        }
    }
}
@Composable private fun EditorShell(controller:EditorController,editor:EditorState,dpiText:String,quitting:Boolean,action:(Command)->Unit,saveSelection:(SelectionSaveTicket)->Unit){
    val eraserState by controller.erasers.state.collectAsState()
    val selectionState by controller.selections.state.collectAsState()
    Surface(Modifier.fillMaxSize()){Column{
        Row(Modifier.fillMaxWidth().background(Color(0xff15202c)).padding(horizontal=18.dp,vertical=12.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)){
            Text("VW",color=accent,fontWeight=FontWeight.Bold,style=MaterialTheme.typography.h5)
            Column(Modifier.weight(1f)){Text(editor.document?.title?:"Visual Workbench",fontWeight=FontWeight.SemiBold);Text(editor.document?.let{"${it.width} × ${it.height} px · revision ${it.render.revision.hostSeq}"}?:"A precise workspace for shared visual work",style=MaterialTheme.typography.caption,color=Color(0xff9cabbc))}
            TextButton(onClick=controller::openAi,enabled=editor.document!=null&&!editor.busy){Text("AI edit")}
            commandButton(Command.Import,action);commandButton(Command.Undo,action,enabled=editor.document?.render?.revision?.canUndo==true);commandButton(Command.Redo,action,enabled=editor.document?.render?.revision?.canRedo==true);commandButton(Command.Export,action,enabled=editor.document!=null)
        }
        AiResultStatus(controller)
        Row(Modifier.weight(1f)){
            Column(Modifier.width(178.dp).fillMaxHeight().background(Color(0xff141e29)).padding(12.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(6.dp)){
                Text("TOOLS",style=MaterialTheme.typography.overline,color=Color(0xff9cabbc));for(c in listOf(Command.Select,Command.Pan,Command.Pen,Command.Rectangle,Command.Ellipse,Command.Line,Command.Arrow,Command.Text,Command.Callout))commandButton(c,action,Modifier.fillMaxWidth(),selected=c.name==editor.tool.name&&!eraserState.active&&!selectionState.active)
                EraserPanel(controller.erasers,controller::chooseEraser,controller::stopErasing,Modifier.fillMaxWidth())
                SelectionPanel(controller.selections,editor.selected.singleOrNull(),saveSelection,Modifier.fillMaxWidth())
                Spacer(Modifier.height(18.dp));Text("LAYERS & OBJECTS",style=MaterialTheme.typography.overline,color=Color(0xff9cabbc))
                editor.document?.layers?.forEach{layer->Text(layer.name,Modifier.padding(top=8.dp),fontWeight=FontWeight.Medium);editor.document?.render?.items?.filter{it.layerId==layer.id}?.forEach{item->Text(item.shape.javaClass.simpleName,Modifier.fillMaxWidth().clickable(enabled=!item.locked){controller.select(item.objectId)}.background(if(item.objectId in editor.selected)Color(0xff294a49)else Color.Transparent).padding(8.dp),style=MaterialTheme.typography.body2)}}
            }
            Box(Modifier.weight(1f).fillMaxHeight()){
                EditorCanvas(controller,editor)
                if(editor.canvasReady&&editor.background!=null)EraserOverlay(controller.erasers,editor.view.camera,editor.document?.render?.revision,editor.document?.documentId)
                if(editor.canvasReady&&editor.background!=null)SelectionOverlay(controller.selections,editor.view.camera,editor.document?.render?.revision,editor.document?.documentId)
                if(editor.document!=null&&(editor.renderIssue!=null||!editor.canvasReady||editor.background==null)){
                    val status=editor.renderIssue?:if(!editor.canvasReady)"Preparing this revision…"else if(editor.busy)"Loading the original image…"else"The source image is unavailable. Review its color and memory settings."
                    Text(status,Modifier.align(Alignment.Center).widthIn(max=440.dp).padding(24.dp),color=Color(0xffaebccb))
                }
                if(editor.document==null)Column(Modifier.align(Alignment.Center).widthIn(max=420.dp).padding(24.dp),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(18.dp)){
                    Text("Make the detail clear.",style=MaterialTheme.typography.h4,fontWeight=FontWeight.Medium);Text("Open an image to annotate, refine and export at its original resolution.",color=Color(0xffaebccb));Button(onClick={action(Command.Import)}){Text("Import image  ·  Ctrl+I")};TextButton(onClick={action(Command.Open)}){Text("Open a saved project")}
                };if(editor.busy||quitting)LinearProgressIndicator(Modifier.fillMaxWidth().align(Alignment.TopCenter))
            }
            Column(Modifier.width(390.dp).fillMaxHeight().background(Color(0xff141e29)).padding(16.dp),verticalArrangement=Arrangement.spacedBy(14.dp)){
                Text("PROPERTIES",style=MaterialTheme.typography.overline,color=Color(0xff9cabbc));Text(if(editor.selected.isEmpty())"New marks"else"${editor.selected.size} selected",fontWeight=FontWeight.Medium)
                Row(horizontalArrangement=Arrangement.spacedBy(6.dp)){palette.forEach{rgba->Box(Modifier.size(24.dp).background(Color(((rgba and 255u)shl 24 or(rgba shr 8)).toInt()),RoundedCornerShape(5.dp)).border(if(editor.color==rgba)2.dp else 0.dp,Color.White,RoundedCornerShape(5.dp)).clickable{controller.color(rgba)})}}
                Text("Width  ${"%.1f".format(editor.width)} px",style=MaterialTheme.typography.body2);Slider(value=editor.width.toFloat(),onValueChange={controller.width(it.toDouble(),false)},onValueChangeFinished={controller.commitWidth()},valueRange=.25f..256f,enabled=!editor.busy)
                Text("Drag selected objects to move. Drag the lower-right handle to resize. Each gesture saves one undo step.",style=MaterialTheme.typography.caption,color=Color(0xff9cabbc));Divider();Text("LOCAL VIEW",style=MaterialTheme.typography.overline,color=Color(0xff9cabbc));commandButton(Command.Fit,action);commandButton(Command.ActualPixels,action);commandButton(Command.MatchPeer,action)
                if(editor.needsColorConsent)Button(onClick={controller.assumeSrgb()}){Text("Treat untagged colors as sRGB")}
                commandButton(Command.Pairing,action);if(editor.sync?.blocked?.let{it>0u}==true)Text("${editor.sync?.blocked} pending edits need conflict review.",color=MaterialTheme.colors.secondary,style=MaterialTheme.typography.caption)
                if(editor.routeWarnings.isNotEmpty())Text("Tether default-route warning. Open Devices for the metric fix and revert.",color=MaterialTheme.colors.secondary,style=MaterialTheme.typography.caption)
                InstructionsPanel(controller.instructionEditor,controller.instructionFocus,controller.semanticEditor,editor.selected.firstOrNull(),Modifier.weight(1f),onTextFocus={controller.canvasFocus(false)})
            }
        }
        Row(Modifier.fillMaxWidth().background(Color(0xff18222e)).padding(10.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)){
            Text(if(editor.exportReady)editor.exportReceipt?.let{"TRANSFER SHELF · ${it.revision} · ${it.width} × ${it.height} px · ${it.request.settings.encoding.label}"}?:"TRANSFER SHELF" else "TRANSFER SHELF · prepare an export for this revision",Modifier.weight(1f),style=MaterialTheme.typography.caption,color=Color(0xff9cabbc))
            NativeFileDragButton(controller,editor.dragPrepared&&editor.exportReady&&!editor.busy,Modifier.width(200.dp).height(38.dp))
            TextButton(onClick=controller::showExport,enabled=editor.document!=null){Text("Export options")}
        }
        editor.message?.let{message->Row(Modifier.fillMaxWidth().background(Color(0xff263749)).padding(12.dp),verticalAlignment=Alignment.CenterVertically){Text(message,Modifier.weight(1f));TextButton(onClick={controller.clearMessage()}){Text("Dismiss")}}}
        Row(Modifier.fillMaxWidth().background(Color(0xff0f1721)).padding(horizontal=16.dp,vertical=8.dp),horizontalArrangement=Arrangement.spacedBy(18.dp),verticalAlignment=Alignment.CenterVertically){Text(syncLabel(editor.sync,editor.sessionRole?.pending?:0u),color=if(editor.sync?.status==SyncStatus.Synced)accent else Color(0xffe8bb75),style=MaterialTheme.typography.caption);Text("INPUT PAUSED",style=MaterialTheme.typography.caption,color=Color(0xff9cabbc));if(editor.exportReady)Text("EXPORT READY",color=accent,style=MaterialTheme.typography.caption);Spacer(Modifier.weight(1f));Text("${"%.0f".format(editor.view.camera.scale*100)}%",style=MaterialTheme.typography.caption);Text(dpiText,style=MaterialTheme.typography.caption)}
    }}
}
private fun WindowMode.placement()=when(this){WindowMode.Windowed->WindowPlacement.Floating;WindowMode.Maximized->WindowPlacement.Maximized;WindowMode.Fullscreen->WindowPlacement.Fullscreen}
private fun WindowPlacement.mode()=when(this){WindowPlacement.Floating->WindowMode.Windowed;WindowPlacement.Maximized->WindowMode.Maximized;WindowPlacement.Fullscreen->WindowMode.Fullscreen}
private fun restoredPosition(value:SavedWindow):WindowPosition{val x=value.x?:return WindowPosition(Alignment.Center);val y=value.y?:return WindowPosition(Alignment.Center);val visible=GraphicsEnvironment.getLocalGraphicsEnvironment().screenDevices.any{it.defaultConfiguration.bounds.contains(x.toDouble()+80,y.toDouble()+40)};return if(visible)WindowPosition(x.dp,y.dp)else WindowPosition(Alignment.Center)}
@Composable private fun commandButton(command:Command,action:(Command)->Unit,modifier:Modifier=Modifier,enabled:Boolean=true,selected:Boolean=false){TooltipArea(tooltip={Surface(elevation=6.dp){Text("${command.label}  ${command.shortcut}",Modifier.padding(8.dp))}}){TextButton(onClick={action(command)},modifier=modifier.background(if(selected)Color(0xff294a49)else Color.Transparent,RoundedCornerShape(6.dp)),enabled=enabled){Text(command.label.removeSuffix(" tool"),style=MaterialTheme.typography.body2)}}}
internal fun shortcutForFocus(e:KeyEvent,canvasFocused:Boolean,modalText:Boolean):Command?{
    if(e.type!=KeyEventType.KeyDown)return null
    // Shift+Tab always leaves the canvas for ordinary keyboard focus traversal.
    if(e.key==Key.Tab&&e.isShiftPressed&&!e.isCtrlPressed&&!e.isAltPressed)return null
    // Ctrl+V belongs to a focused text field/control unless the canvas owns it.
    if(e.isCtrlPressed&&!e.isAltPressed&&e.key==Key.V&&!canvasFocused)return null
    if(!shortcutAllowed(canvasFocused,modalText,e.isCtrlPressed||e.isAltPressed,e.key==Key.F11||e.key==Key.F12))return null
    return shortcut(e)
}
internal fun shortcut(e:KeyEvent):Command?{
    if(e.isCtrlPressed&&e.isAltPressed)return when(e.key){Key.One->Command.Windowed;Key.Two->Command.Maximized;Key.F->Command.FollowPeer;Key.M->Command.MatchPeer;Key.V->Command.PeerOutline;else->null}
    if(e.isCtrlPressed)return when(e.key){Key.I->Command.Import;Key.O->Command.Open;Key.E->Command.Export;Key.W->Command.Close;Key.Z->if(e.isShiftPressed)Command.Redo else Command.Undo;Key.Y->Command.Redo;Key.A->Command.SelectAll;Key.One->Command.ActualPixels;Key.P->Command.Pairing;Key.Comma->Command.Settings;Key.C->if(e.isShiftPressed)Command.Copy else null;Key.V->Command.Paste;else->null}
    if(e.isAltPressed)return if(e.key==Key.F4)Command.Quit else null
    return when(e.key){Key.V->Command.Select;Key.H->Command.Pan;Key.P->Command.Pen;Key.R->Command.Rectangle;Key.E->Command.Ellipse;Key.L->Command.Line;Key.A->Command.Arrow;Key.T->Command.Text;Key.M->Command.Callout;Key.F->Command.Fit;Key.Equals->Command.ZoomIn;Key.Minus->Command.ZoomOut;Key.F11->Command.Fullscreen;Key.F12->Command.Diagnostics;Key.Escape->Command.ClearSelection;Key.Delete,Key.Backspace->Command.Delete;Key.Tab->Command.CycleMarker;Key.Enter->Command.Instructions;Key.C->Command.Color;Key.RightBracket->Command.WidthUp;Key.LeftBracket->Command.WidthDown;Key.DirectionLeft->Command.NudgeLeft;Key.DirectionRight->Command.NudgeRight;Key.DirectionUp->Command.NudgeUp;Key.DirectionDown->Command.NudgeDown;else->null}
}
private fun dispatch(c:Command,shift:Boolean,editor:EditorController,mode:(WindowMode)->Unit,dialog:(Command)->Unit,quit:()->Unit){when(c){Command.Import,Command.Open,Command.Pairing,Command.Settings,Command.Diagnostics->dialog(c);Command.Instructions->editor.editInstruction();Command.Export,Command.Copy->editor.showExport();Command.Paste->editor.showPaste();Command.Close->editor.closeProject();Command.Quit->quit();Command.Undo->editor.undo(false);Command.Redo->editor.undo(true);Command.Delete->editor.delete();Command.SelectAll->editor.selectAll();Command.ClearSelection->editor.clearSelection();Command.Select->editor.tool(Tool.Select);Command.Pan->editor.tool(Tool.Pan);Command.Pen->editor.tool(Tool.Pen);Command.Rectangle->editor.tool(Tool.Rectangle);Command.Ellipse->editor.tool(Tool.Ellipse);Command.Line->editor.tool(Tool.Line);Command.Arrow->editor.tool(Tool.Arrow);Command.Text->editor.tool(Tool.Text);Command.Callout->editor.tool(Tool.Callout);Command.Fit->editor.fit();Command.ActualPixels->editor.actualPixels();Command.ZoomIn->editor.zoom(1.25);Command.ZoomOut->editor.zoom(.8);Command.Windowed->mode(WindowMode.Windowed);Command.Maximized->mode(WindowMode.Maximized);Command.Fullscreen->mode(WindowMode.Fullscreen);Command.FollowPeer->editor.follow();Command.MatchPeer->editor.matchPeer();Command.PeerOutline->editor.outline();Command.CycleMarker->editor.cycleMarker();Command.Color->editor.color(palette[(palette.indexOf(editor.state.value.color)+1)%palette.size]);Command.WidthUp->editor.width(editor.state.value.width+1,true);Command.WidthDown->editor.width(editor.state.value.width-1,true);Command.NudgeLeft->editor.nudge(if(shift)-10.0 else -1.0,0.0);Command.NudgeRight->editor.nudge(if(shift)10.0 else 1.0,0.0);Command.NudgeUp->editor.nudge(0.0,if(shift)-10.0 else -1.0);Command.NudgeDown->editor.nudge(0.0,if(shift)10.0 else 1.0)}}
