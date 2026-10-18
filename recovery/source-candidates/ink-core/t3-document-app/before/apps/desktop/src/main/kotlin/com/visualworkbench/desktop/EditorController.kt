package com.visualworkbench.desktop

import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import com.visualworkbench.shared.*
import com.visualworkbench.bindings.host.HostService
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.nio.file.Path
import javax.imageio.ImageIO
import org.jetbrains.skia.Image

data class EditorState(val document:DocumentSnapshot?=null,val background:ImageBitmap?=null,val view:ViewState=ViewState(),val selected:Set<String> = emptySet(),val tool:Tool=Tool.Select,val preview:Map<String,Transform> = emptyMap(),val draft:Pair<Point,Point>?=null,val busy:Boolean=false,val message:String?=null,val textAnchor:Point?=null,val color:UInt=0x78dcc8ffu,val width:Double=3.0,val path:Path?=null,val needsColorConsent:Boolean=false,val projectEpoch:Long=0,val canvasReady:Boolean=false,val canvasFocused:Boolean=false,val renderIssue:String?=null,
    val exportSettings:ExportSettings=ExportSettings(),val exportOpen:Boolean=false,val pasteOpen:Boolean=false,
    val exportCheck:ExportCheck?=null,val exportReceipt:CompletedExport?=null,val dragPrepared:Boolean=false,
    val sync:SessionStatus?=null,val peerFrame:PeerFrame?=null,val peerViewport:PeerViewport?=null,
    val routeWarnings:List<TetherRouteWarning> = emptyList(),val previewStyles:Map<String,ObjectStyle> = emptyMap(),
    val wetStroke:RenderItem?=null,val shareViewport:Boolean=false,val sessionRole:ProjectSessionRole?=null) {
    val exportReady:Boolean get()=exportReceipt?.request?.matches(this)==true
}
private data class Drag(val start:Point,val initial:ViewState,val selected:List<RenderItem>,val bounds:Rect?,val resize:Boolean,val pan:Boolean,val binding:RenderBinding,val live:ObjectPreviewLease?=null,val newObject:NewObjectPreviewLease?=null,val tool:Tool=Tool.Select)
private data class WidthGesture(val binding:RenderBinding,val items:List<RenderItem>,val live:ObjectPreviewLease?)
private data class CachedExport(val workspace:TransferWorkspace,val receipt:CompletedExport)
class EditorController internal constructor(private val scope:CoroutineScope,val core:WorkbenchCore,private val handoff:DesktopHandoff,val location:LocationPolicy,private val deviceId:String,workRoot:Path=location.path.resolve(".handoff")) {
    constructor(scope:CoroutineScope,core:WorkbenchCore,host:HostService,location:LocationPolicy,deviceId:String,workRoot:Path):this(scope,core,NativeDesktopHandoff(host),location,deviceId,workRoot)
    private val mutable=MutableStateFlow(EditorState());val state:StateFlow<EditorState> = mutable
    private var project:WorkbenchProject?=null;private var changes:Job?=null;private val edits=Mutex();private var drag:Drag?=null
    private var closing=false;private var queued=0;private var colorAssumed=false;private var generation=0L
    private val transferFiles=TransferFiles(workRoot);private var transferJob:Job?=null;private var cachedExport:CachedExport?=null
    private var preparedDrag:Pair<CompletedExport,DesktopDragFile>?=null
    private var widthGesture:WidthGesture?=null
    private var ink:DesktopInkRun?=null;private var inkWork:Pair<DesktopInkRun,Job>?=null
    private var refreshTicket=0L;private var changeSequence=0uL
    private var sessionWork:Job?=null;private var sessionProvider:DesktopSessionController?=null
    private var wetCommitted:String?=null
    private val live=DesktopLiveLink(scope,{mutable.value.document?.binding(generation)},::sessionStatus,
        {frame->update{it.copy(peerFrame=frame)}},::report,::detachInstructionFocus)
    internal fun publishAgentCaptureStatus(value:LocalAgentCaptureSummary) {
        val attached=live.attachment()?:return
        try { agentCaptureStatus(attached.link).publish(value) }
        catch (_:SessionFailure) { /* Native peer status remains Unknown; no authority is inferred. */ }
    }
    private var viewportJob:Job?=null
    private var aiDirectory: Path? = null
    private var aiRetiring = false
    internal fun useAiDirectory(path: Path) { check(aiDirectory == null); aiDirectory = path }
    private fun aiAttachment(): AiAttachment? = if(aiRetiring||closing)null else project?.let { p -> mutable.value.document?.let { AiAttachment(p,it,generation) } }
    private fun aiPoses(): Map<String,Transform> {
        val current=mutable.value
        return current.peerFrame?.takeIf{it.binding==current.document?.binding(generation)}?.previews?.items
            ?.asSequence()?.filter{it.shape is Shape.Result}?.take(9)?.associate{it.objectId to it.transform}.orEmpty()+current.preview
    }
    internal val aiService=AiServiceController(scope, "") { _, initialize ->
        createAiService((aiDirectory ?: throw AiFailure(AiFailureKind.Unsupported)).toString(), initialize)
    }
    internal val ai: AiEditorController<AiBitmap> = AiEditorController(scope,core,aiService,::aiAttachment,edits,
        metadata={ owned -> val info=owned.project.info();val now=System.currentTimeMillis();WorkflowMetadata(core.newId(now.toULong()),deviceId,info.nextLamport,now) },
        refreshAfterMutation={ if(it.sameProject(aiAttachment()))refresh() },image=::aiReadBitmap,abandonImage=AiBitmap::abandon,
        canOperate={!closing&&!mutable.value.busy&&!instructionEditor.state.value.busy&&!semanticEditor.state.value.busy})
    internal val aiResults=AiResultController(scope,core,::aiAttachment,{mutable.value.view.camera},::aiPoses,
        image=::aiReadBitmap,abandonImage=AiBitmap::abandon)
    internal fun aiResultFrame(): AiResultFrame<AiBitmap>? = aiResults.state.value.frame?.takeIf{it.matches(aiAttachment(),mutable.value.view.camera,aiPoses())}
    internal fun aiResultReady(): Boolean = mutable.value.document?.render?.items?.none{it.shape is Shape.Result} != false || aiResultFrame()!=null
    fun openAi() {
        val current=mutable.value
        if(closing||current.busy||current.document==null||current.textAnchor!=null||current.exportOpen||current.pasteOpen)return
        if(instructionEditor.hasUnsaved()){report("Save or discard the instruction draft before opening AI review.");return}
        cancelInput();ai.open();instructionInteractionChanged()
    }
    fun hideAi(){ai.hide();instructionInteractionChanged()}
    fun mayCloseAi():Boolean{
        if(!ai.needsDecision())return true
        report("Save the paid Result or explicitly discard its review before closing or replacing this project.")
        if(!instructionEditor.hasUnsaved()){ai.open();instructionInteractionChanged()}
        return false
    }
    private suspend fun retireAi(){
        aiRetiring=true
        try{if(closing)ai.close()else ai.detach()}
        finally{try{if(closing)aiResults.close()else aiResults.detach()}finally{if(closing)aiService.close()}}
    }
    private data class MarkerPress(val binding: RenderBinding, val first: Point, val camera: Camera, val invertSnapping: Boolean, var end: Point = first)
    private var markerPress: MarkerPress? = null
    val instructionEditor = InstructionEditor(scope, edits,
        { val p = project; val d = mutable.value.document; if (p != null && d != null && !closing) InstructionAttachment(p, d) else null },
        { core.newId(System.currentTimeMillis().toULong()) },
        { p -> val info = p.info(); val now = System.currentTimeMillis(); WorkflowMetadata(core.newId(now.toULong()), deviceId, info.nextLamport, now) },
        { refresh() }, InstructionEntryMethod.PcKeyboard, localFocused = ::localInstructionFocused)
    val semanticEditor = SemanticEditor(scope, edits,
        { val p = project; val d = mutable.value.document; if (p != null && d != null && !closing) InstructionAttachment(p, d) else null },
        { val s = mutable.value; !closing && !s.busy && !ai.state.value.open && queued == 0 && drag == null && ink == null &&
            !instructionEditor.hasUnsaved() && !instructionEditor.state.value.busy && instructionAdmission.allows(s.textAnchor != null, s.exportOpen, s.pasteOpen) },
        { p -> val info = p.info(); val now = System.currentTimeMillis(); WorkflowMetadata(core.newId(now.toULong()), deviceId, info.nextLamport, now) },
        { refresh() },
        { placement -> instructionEditor.place(placement.point, placement.style, placement.bounds, placement.elementEids,
            placement.snapshotId, placement.binding) }, interactionChanged = ::instructionInteractionChanged)
    private val instructionAdmission = com.visualworkbench.desktop.instructions.InstructionFocusAdmission { instructionInteractionChanged() }
    val instructionFocus = InstructionFocusController(scope, instructionEditor,
        { val p = project; val d = mutable.value.document; if (p != null && d != null && !closing) InstructionAttachment(p, d) else null },
        { !closing && !mutable.value.busy && !semanticEditor.state.value.busy && drag == null && ink == null && widthGesture == null &&
            !ai.state.value.open && instructionAdmission.allows(mutable.value.textAnchor != null, mutable.value.exportOpen, mutable.value.pasteOpen) },
        { id, peer -> update { it.copy(selected = setOf(id), canvasFocused = if (peer) false else it.canvasFocused) } },
        selectedObject = { mutable.value.selected.singleOrNull() })
    private fun localInstructionFocused(field: InstructionField) { instructionFocus.localField(field) }
    private suspend fun detachInstructionFocus() { instructionFocus.detach() }
    private fun instructionInteractionChanged() { instructionFocus.interactionChanged() }
    fun instructionModal(value: Boolean) { instructionAdmission.modal = value }
    fun instructionPicker(): AutoCloseable = instructionAdmission.picker()
    fun editInstruction() { cancelInput(); mutable.value.selected.firstOrNull()?.let(instructionEditor::focusObject) ?: instructionEditor.focusGlobal() }
    fun mayCloseInstructions(): Boolean {
        if (!instructionEditor.hasUnsaved()) return true
        report("Save or discard the instruction draft before closing or replacing this project."); return false
    }
    fun sealInstructionsForClose(): Boolean {
        if (instructionEditor.sealIfClean()) return true
        report("Save or discard the instruction draft before closing or replacing this project."); return false
    }
    private var selectionPointer=false
    private var eraserPointer=false
    internal val selections=SelectionController(scope,core,workRoot.resolve("selections"),{
        if(!closing&&project!=null)refresh()
    })
    internal val erasers=EraserController(core,scope,selections){receipt->
        if(!closing&&project!=null){
            if(receipt!=null)update{current->current.copy(selected=current.selected.mapNotNull{id->
                if(id in receipt.replacements)receipt.replacements[id] else id.takeIf{it !in receipt.removed}
            }.toSet())}
            refresh()
        }
    }
    fun chooseEraser(value:EraserChoice){cancelInput();erasers.tools.choose(value,mutable.value.selected.singleOrNull());update{it.copy(textAnchor=null)}}
    fun stopErasing(){cancelInput();erasers.tools.drawing()}
    private fun update(f:(EditorState)->EditorState){val before=mutable.value;var next=f(before);if(preparedDrag?.first?.request?.matches(next)==false){preparedDrag?.second?.close();preparedDrag=null;next=next.copy(dragPrepared=false)};if(next.exportCheck?.request?.matches(next)==false)next=next.copy(exportCheck=null);mutable.value=next;if(next.document!==before.document)ai.changed();if(next.document!==before.document||next.view.camera!=before.view.camera||next.preview!=before.preview||next.peerFrame!=before.peerFrame)aiResults.request();selections.viewport(next.view.camera);erasers.viewport(next.view.camera);if(next.shareViewport&&(next.view.camera!=before.view.camera||!before.shareViewport))scheduleViewport()}
    internal suspend fun compileForMcp(choice:com.visualworkbench.desktop.mcp.McpCompileChoice):WorkbenchCompiledPackage = edits.withLock {
        val view=mutable.value;val document=view.document?:throw PackageFailure(PackageFailureKind.Invalid)
        if(closing||view.busy||ai.state.value.open||semanticEditor.state.value.busy||instructionEditor.state.value.busy||drag!=null||ink!=null||inkWork!=null||widthGesture!=null||instructionEditor.hasUnsaved())throw PackageFailure(PackageFailureKind.Busy)
        val current=project?:throw PackageFailure(PackageFailureKind.Closed)
        val expected=document.render.revision
        val epoch=generation;val actual=current.info()
        if(closing||project!==current||generation!=epoch||mutable.value.document?.documentId!=document.documentId)throw PackageFailure(PackageFailureKind.Stale)
        if(actual.projectId!=expected.projectId||actual.hostSeq!=expected.hostSeq||actual.stateHash!=expected.stateHash)throw PackageFailure(PackageFailureKind.Stale)
        current.compilePackage(PackageCompileOptions(WorkflowBinding(actual.projectId,document.documentId,actual.hostSeq,actual.stateHash),
            core.newId(System.currentTimeMillis().toULong()),System.currentTimeMillis(),choice.target,choice.semanticSnapshotId,choice.includeWindowTitle,choice.assumeSrgb,choice.allowDepth))
    }
    fun clearMessage(){update{it.copy(message=null)}}
    internal fun useSessionService(value:DesktopSessionController){sessionProvider=value}
    /** Capture the destination BEFORE the hotkey worker begins. A later project,
     * link or visible revision can never silently become the receiver. */
    internal fun captureDestination(): suspend (WorkbenchProject, Path, String?) -> Unit {
        val target = project
        val attached = live.attachment()
        val epoch = generation
        val snapshot = mutable.value.document
        val expected = snapshot?.render?.revision?.let { WorkflowBinding(it.projectId, snapshot.documentId, it.hostSeq, it.stateHash) }
        val eligible = target != null && attached != null && expected != null && mutable.value.sessionRole?.isHost == true &&
            attached.status.status in setOf(SyncStatus.Syncing, SyncStatus.Synced)
        return { captured, path, warning ->
            try {
                if (!eligible) {
                    report("Lossless capture saved at $path. Open and connect a host project to send a new capture to the phone.")
                } else edits.withLock {
                    if (closing || project !== target || generation != epoch || live.attachment() !== attached ||
                        attached!!.status.status !in setOf(SyncStatus.Syncing, SyncStatus.Synced)) {
                        report("The capture destination changed. The unchanged lossless capture remains at $path; it was not redirected.")
                        return@withLock
                    }
                    // Appending does not replace the current document, drafts,
                    // camera, link, provisional stroke or local selection.
                    val sourceInfo = captured.info()
                    val sourceDoc = sourceInfo.documentIds.single()
                    val destination = checkNotNull(target)
                    val actual = destination.info()
                    val now = System.currentTimeMillis()
                    val plan = prepareCaptureDelivery(captured, destination,
                        WorkflowBinding(sourceInfo.projectId, sourceDoc, sourceInfo.hostSeq, sourceInfo.stateHash),
                        checkNotNull(expected), WorkflowMetadata(core.newId(now.toULong()), actual.deviceId, actual.nextLamport, now))
                    try {
                        if (closing || project !== target || generation != epoch || live.attachment() !== attached) throw CancellationException("Capture destination retired")
                        plan.commit()
                        report("Lossless capture added to this paired project. The phone opens it after verifying the original; active drafts stay in place.${warning?.let { " $it" }.orEmpty()}")
                    } finally { plan.close() }
                }
            } catch (cancel: CancellationException) {
                report("Capture delivery cancelled. The lossless recovery project remains at $path; inspect the paired project before retrying a published edit.")
                throw cancel
            } catch (_: Exception) {
                report("Capture delivery could not finish or its destination revision changed. The lossless recovery project remains at $path.")
            } finally { withContext(NonCancellable) { captured.close() } }
        }
    }

    fun report(message:String){update{it.copy(message=message)}}
    private fun task(epoch: Long? = null, block: suspend () -> Unit): Job? {
        if (closing) return null
        if (queued >= 32) { report("The edit queue is full. Wait for current work to finish."); return null }
        queued++
        return scope.launch {
            try {
                edits.withLock {
                    if (closing) return@withLock
                    if (epoch != null && epoch != generation) {
                        report("The project changed. The queued action was cancelled."); return@withLock
                    }
                    update { it.copy(busy = true, message = null) }
                    try { block() }
                    catch (cancel: CancellationException) {
                        if (!closing) report(if (cancel is RetainedDropCancellation)
                            "Import cancelled. The accepted drop's original bytes remain at ${cancel.input}. See drop-recovery.txt beside it for the original name and retry steps."
                            else "Cancellation settled. Any file or clipboard copy already published remains in its destination.")
                        throw cancel
                    } catch (error: Exception) {
                        report(when (error) {
                            is HandoffFailure, is TransferFailure, is java.nio.file.FileAlreadyExistsException -> transferMessage(error)
                            is CoreFailure -> "Core ${error.kind.name.lowercase()}. The last accepted revision is preserved."
                            is SessionFailure -> sessionMessage(error)
                            is UnsupportedOperationException -> error.message ?: "This document feature is unavailable."
                            else -> "The operation could not finish. Check diagnostics and retry."
                        })
                    } finally { update { it.copy(busy = false) } }
                }
            } finally { queued-- }
        }
    }
    private fun transfer(epoch:Long,block:suspend()->Unit):Job?{if(transferJob?.isActive==true){report("A transfer is already running. Finish or cancel it first.");return null};val job=task(epoch,block);transferJob=job;return job}
    private suspend fun discardExport(){preparedDrag?.second?.close();preparedDrag=null;val prior=cachedExport;cachedExport=null;update{it.copy(exportReceipt=null,exportCheck=null,dragPrepared=false)};prior?.workspace?.close()}
    private suspend fun attach(value:WorkbenchProject,path:Path){var adopted=false;var instructionSeal=false;try{if(!sealInstructionsForClose())return;instructionSeal=true;if(!mayCloseAi())return;val info=value.info();val doc=info.documentIds.firstOrNull()?:error("Project has no document");val snapshot=value.document(doc);validateDrawing(snapshot);cancelInput();live.close();changes?.cancelAndJoin();discardExport();semanticEditor.detach();instructionEditor.detach();erasers.detach();selections.detach();retireAi();project?.close();project=null;if(closing)return;generation++;changeSequence=0uL;refreshTicket++;project=value;aiRetiring=false;adopted=true;colorAssumed=false;update{EditorState(path=path,view=it.view.copy(peer=null),document=snapshot,projectEpoch=generation,canvasFocused=it.canvasFocused,busy=true,exportSettings=ExportSettings(encoding=if(snapshot.bitDepth>8u)ExportEncoding.Png16 else ExportEncoding.Png8))};selections.bind(value,snapshot,generation,changeSequence,mutable.value.view.camera);erasers.bind(value,snapshot,generation,changeSequence,mutable.value.view.camera);instructionEditor.refresh();instructionFocus.revisionChanged();semanticEditor.refresh();fit();loadBackground();refreshRole(value,generation)
        changes=scope.launch{try{value.changes.collect{event->if(project===value&&event.sequence>changeSequence){erasers.observe(event);selections.observe(event);changeSequence=event.sequence;val current=mutable.value.document?.render?.revision;if(current!=null&&(event.project.hostSeq!=current.hostSeq||event.project.stateHash!=current.stateHash||event.project.nextLamport!=current.nextLamport))refresh()}}}catch(cancel:CancellationException){throw cancel}catch(_:Exception){report("The saved revision could not be refreshed. Reopen the project to retry.")}}
        }finally{withContext(NonCancellable){
            try { if(!adopted)value.close() }
            finally { if(instructionSeal&&!closing){instructionEditor.detach();instructionEditor.resume();semanticEditor.resumeAfterReplacement();instructionEditor.refresh();instructionFocus.revisionChanged()} }
        }}
    }
    private suspend fun loadBackground(){val p=project?:return;val doc=mutable.value.document?:return;try{val pixels=p.background(doc.documentId,256uL*1024uL*1024uL,assumeUntaggedSrgb=colorAssumed);val image=withContext(Dispatchers.Default){bitmap(pixels)};if(project===p)update{it.copy(background=image,needsColorConsent=false)}}catch(error:CoreFailure){update{it.copy(needsColorConsent=error.kind==CoreFailureKind.Raster&&!colorAssumed)};report("The original is saved unchanged. Full-resolution display needs a supported color profile and enough memory within 256 MiB. For untagged images, explicitly choose the sRGB assumption.")}}
    fun assumeSrgb(){task(generation){colorAssumed=true;loadBackground()}}
    private fun validateDrawing(snapshot:DocumentSnapshot){if(snapshot.render.items.any{it.shape is Shape.Adjustment})throw UnsupportedOperationException("This document contains adjustment objects that the desktop canvas cannot display yet. Its original and saved edits are preserved.")}
    fun importImage(path:Path)=importFile(ImportRequest(generation,path.toAbsolutePath().normalize()))
    fun importFile(request: ImportRequest, onStaged: (Boolean) -> Unit = {}, retainAcceptedDrop: Boolean = false): Job? {
        val completed = java.util.concurrent.atomic.AtomicBoolean(false)
        fun finish(value: Boolean) { if (completed.compareAndSet(false, true)) onStaged(value) }
        val job = transfer(request.epoch) {
            val workspace = transferFiles.create()
            var retained = false
            var imported = false
            try {
                val input = workspace.stage(request.path)
                if (retainAcceptedDrop) { workspace.markAcceptedDrop(request.path.fileName.toString()); retained = true }
                finish(true)
                if (request.path.fileName.toString().endsWith(".mp4", true)) {
                    val p = project ?: throw HandoffFailure("Open a project before attaching an MP4 asset.")
                    val files = p as? WorkbenchFileTransfers ?: throw HandoffFailure("File transfers are unavailable in this core binding.")
                    val info = p.info(); val now = System.currentTimeMillis()
                    files.attachFile(AttachFileOptions(core.newId(now.toULong()), info.deviceId, info.nextLamport, now,
                        input.toString(), workspace.directory.toString(), FileAssetKind.Mp4))
                    refresh(); report("MP4 stored as an ordinary project asset. No timeline was created.")
                } else createImported(input, workspace.directory, request.path.fileName.toString())
                imported = true
            } catch (cancel: CancellationException) {
                if (retained) throw RetainedDropCancellation(workspace.input).also { it.initCause(cancel) }
                throw cancel
            } catch (error: Exception) {
                if (retained) throw HandoffFailure("Import did not complete. The accepted drop's original bytes remain at ${workspace.input}. See drop-recovery.txt beside it for the original name and retry steps.")
                throw error
            } finally { if (!retained || imported) workspace.close() }
        }
        if (job == null) finish(false) else job.invokeOnCompletion { finish(false) }
        return job
    }
    private suspend fun createImported(input:Path,work:Path,title:String){val streaming=core as? WorkbenchStreamingCore?:throw HandoffFailure("Streaming import is unavailable in this core binding.");val time=System.currentTimeMillis();val id=core.newId(time.toULong());val destination=location.path.toAbsolutePath().resolve("$id.vwb");withContext(Dispatchers.IO){Files.createDirectories(location.path)};val value=try{streaming.createFile(CreateFileProject(destination.toString(),id,core.newId(time.toULong()),core.newId(time.toULong()),deviceId,title.take(256),input.toString(),work.toString(),time))}catch(error:CoreFailure){if(error.kind==CoreFailureKind.Raster)throw HandoffFailure("Image import refused. Images over 50 MP, unsupported profiles and sources exceeding the 256 MiB memory budget are not accepted; the original was not resized.");throw error};attach(value,destination)}
    fun showPaste(){if(!mutable.value.busy)update{it.copy(pasteOpen=true)}}
    fun dismissPaste(){update{it.copy(pasteOpen=false)}}
    fun pasteImage(assumeSrgb:Boolean,epoch:Long=generation){update{it.copy(pasteOpen=false)};transfer(epoch){val image=handoff.paste(assumeSrgb);val workspace=transferFiles.create();try{val input=workspace.stage(image.png);createImported(input,workspace.directory,"Pasted image")}finally{workspace.close()}}}
    fun acceptDrop(paths:List<Path>,onStaged:(Boolean)->Unit={}):Job?{if(paths.size!=1){report("Drop one PNG, JPEG, WebP, HEIC or MP4 file at a time.");onStaged(false);return null};val path=paths.single();val extension=path.fileName.toString().substringAfterLast('.',"").lowercase();if(extension !in setOf("png","jpg","jpeg","webp","heic","heif","mp4")){report("This drop supports PNG, JPEG, WebP, admitted HEIC originals and ordinary MP4 assets.");onStaged(false);return null};return importFile(ImportRequest(generation,path.toAbsolutePath().normalize()),onStaged,retainAcceptedDrop=true)}
    fun open(path:Path,epoch:Long=generation){task(epoch){attach(core.open(path.toAbsolutePath().toString()),path)}}
    suspend fun close(){closing=true;transferJob?.cancel();sessionWork?.cancel();viewportJob?.cancel();cancelInput();withContext(NonCancellable){edits.withLock{live.close();changes?.cancelAndJoin();changes=null;discardExport();semanticEditor.detach();instructionEditor.detach();erasers.detach();selections.detach();retireAi();project?.close();project=null;generation++;refreshTicket++;update{EditorState(view=it.view.copy(peer=null),projectEpoch=generation,canvasFocused=it.canvasFocused)}}}}
    fun closeProject(){if(!mayCloseAi())return;if(!mayCloseInstructions())return;cancelInput();sessionWork?.cancel();task(generation){
        // The pre-enqueue check can become stale while another owner holds
        // edits. task has now set busy under that same mutex, fencing Send.
        if(!mayCloseAi())return@task
        if(!sealInstructionsForClose())return@task
        try { live.close();changes?.cancelAndJoin();changes=null;discardExport();semanticEditor.detach();instructionEditor.detach();erasers.detach();selections.detach();retireAi();project?.close();project=null;generation++;refreshTicket++;update{EditorState(view=it.view.copy(peer=null),projectEpoch=generation,canvasFocused=it.canvasFocused)} }
        finally { if(!closing){instructionEditor.detach();instructionEditor.resume();semanticEditor.resumeAfterReplacement();instructionEditor.refresh();instructionFocus.revisionChanged()} }
    }}
    private suspend fun refresh(){
        val p=project?:return;val epoch=generation;val id=mutable.value.document?.documentId?:return
        val ticket=++refreshTicket;val observed=changeSequence;val snapshot=p.document(id)
        if(project!==p||epoch!=generation||ticket!=refreshTicket||observed<changeSequence)return
        // A host sequence is the accepted journal position, not the version of
        // the client's optimistic view. Confirm that this fetch still matches
        // the worker's current visible hash before admitting a same-seq change.
        val currentInfo=p.info()
        if(project!==p||epoch!=generation||ticket!=refreshTicket||observed<changeSequence)return
        if(currentInfo.hostSeq!=snapshot.render.revision.hostSeq||currentInfo.stateHash!=snapshot.render.revision.stateHash)return
        when(snapshotDecision(mutable.value.document,snapshot,allowVisibleChange=true)){
            SnapshotDecision.Stale->return
            SnapshotDecision.Duplicate->{selections.bind(p,snapshot,generation,changeSequence,mutable.value.view.camera);erasers.bind(p,snapshot,generation,changeSequence,mutable.value.view.camera);return}
            SnapshotDecision.Conflict->{cancelDrag();update{it.copy(canvasReady=false,renderIssue="The same revision returned different content. Reopen the project before editing.")};return}
            SnapshotDecision.Newer->{
                val issue=try{validateDrawing(snapshot);null}catch(error:UnsupportedOperationException){error.message}
                cancelDrag();cancelWidth();update{it.copy(document=snapshot,selected=it.selected.intersect(snapshot.render.items.filterNot{item->item.locked}.map{item->item.objectId}.toSet()),preview=emptyMap(),previewStyles=emptyMap(),draft=null,peerFrame=null,exportReceipt=null,exportCheck=null,canvasReady=false,renderIssue=issue)}
            }
        }
        selections.bind(p,snapshot,generation,changeSequence,mutable.value.view.camera);erasers.bind(p,snapshot,generation,changeSequence,mutable.value.view.camera)
        instructionEditor.refresh();instructionFocus.revisionChanged();semanticEditor.refresh()
        refreshRole(p,epoch)
    }
    private suspend fun refreshRole(p:WorkbenchProject,epoch:Long){val provider=sessionProvider?:return;if(!provider.state.value.ready)return;try{val role=provider.requireService().projectRole(p);if(project===p&&generation==epoch)update{it.copy(sessionRole=role)}}catch(cancel:CancellationException){throw cancel}catch(_:SessionFailure){report("The local pending-edit count could not be refreshed.")}}
    private suspend fun options(expected:RenderBinding?=null,gestureId:String?=null,createdAt:Long?=null):EditOptions{
        val p=project?:error("No project");val info=p.info();val document=mutable.value.document?:error("No document")
        if(expected!=null&&(expected.epoch!=generation||expected.projectId!=info.projectId||expected.documentId!=document.documentId||expected.hostSeq!=info.hostSeq||expected.stateHash!=info.stateHash))throw CoreFailure(CoreFailureKind.Invalid)
        return EditOptions(core.newId(System.currentTimeMillis().toULong()),document.documentId,info.deviceId,info.nextLamport,createdAt?:System.currentTimeMillis(),expectedHostSeq=expected?.hostSeq,expectedStateHash=expected?.stateHash,gestureId=gestureId)
    }
    private suspend fun performEdit(commands:List<EditCommand>,binding:RenderBinding,gestureId:String?=null,createdAt:Long?=null,onCommitted:()->Unit={}):Boolean{
        if(commands.isEmpty())return false
        val guides=mutable.value.document?.render?.items?.filter{it.shape is Shape.Guide}?.map{it.objectId}?.toSet().orEmpty()
        if(commands.any{it is EditCommand.SetTransform&&it.objectId in guides}){report("Pixel selections and guides keep their source coordinates. Use selection tools to edit coverage.");return false}
        try{project?.edit(options(binding,gestureId,createdAt),commands)?:error("No project");onCommitted()}catch(error:CoreFailure){if(error.kind==CoreFailureKind.Invalid){refresh();if(mutable.value.document?.binding(generation)!=binding){report("The document changed before this edit could commit. Review it and repeat the gesture.");return false}};throw error}
        refresh();return true
    }
    private fun edit(commands:List<EditCommand>,binding:RenderBinding?=mutable.value.document?.binding(generation),afterSuccess:()->Unit={}){if(commands.isEmpty()||binding==null)return;task(binding.epoch){if(performEdit(commands,binding))afterSuccess()}}
    private fun gestureEdit(commands:List<EditCommand>,binding:RenderBinding,preview:GesturePreview?){
        if(commands.isEmpty()){live.cancelLater(preview?.attachment,preview?.id);return}
        var settled=false
        val job=task(binding.epoch){var committed=false;try{performEdit(commands,binding,preview?.id,(preview as? NewObjectPreviewLease)?.createdAt){committed=true}}finally{settled=true;withContext(NonCancellable){live.finish(preview?.attachment,preview?.id,!committed)}}}
        if(job==null)live.cancelLater(preview?.attachment,preview?.id)
        else job.invokeOnCompletion{if(!settled)live.cancelLater(preview?.attachment,preview?.id)}
    }
    private fun editCurrent(build:(DocumentSnapshot,Set<String>)->List<EditCommand>){val selected=mutable.value.selected;val epoch=generation;task(epoch){refresh();val document=mutable.value.document?:return@task;if(mutable.value.renderIssue!=null)return@task;performEdit(build(document,selected),document.binding(epoch))}}
    internal fun canvasPrepared(binding:RenderBinding){if(geometryMatches(binding,mutable.value))update{val swap=wetCommitted?.let{id->it.document?.render?.items?.any{item->item.objectId==id}}==true;if(swap)wetCommitted=null;it.copy(canvasReady=true,wetStroke=if(swap)null else it.wetStroke)}}
    internal fun canvasFailed(binding:RenderBinding,message:String){if(mutable.value.document?.binding(generation)==binding){cancelDrag();update{it.copy(canvasReady=false,renderIssue=message)}}}
    fun canvasFocus(focused:Boolean){update{it.copy(canvasFocused=focused)}}
    fun tool(value:Tool){cancelInput();erasers.tools.drawing();update{it.copy(tool=value,preview=emptyMap(),draft=null)}}
    fun viewport(width:Double,height:Double){if(width<=0||height<=0)return;update{it.copy(view=it.view.copy(camera=it.view.camera.copy(viewportWidth=width,viewportHeight=height)))}}
    fun fit(){val d=mutable.value.document?:return;update{val c=it.view.camera;it.copy(view=it.view.copy(camera=c.copy(center=Point(d.width.toDouble()/2,d.height.toDouble()/2),scale=minOf(c.viewportWidth/d.width.toDouble(),c.viewportHeight/d.height.toDouble()).coerceAtLeast(0.0001)*0.9,rotation=0.0)))}}
    fun actualPixels(){update{it.copy(view=it.view.copy(camera=it.view.camera.copy(scale=1.0)))}}
    fun zoom(factor:Double,at:Point?=null){val s=mutable.value;val c=s.view.camera;val anchor=at?:Point(c.viewportWidth/2,c.viewportHeight/2);val before=core.mapPoints(c,true,listOf(anchor)).single();val next=c.copy(scale=(c.scale*factor).coerceIn(0.001,256.0));val after=core.mapPoints(next,true,listOf(anchor)).single();update{it.copy(view=it.view.copy(camera=next.copy(center=Point(next.center.x+before.x-after.x,next.center.y+before.y-after.y))))}}
    fun pan(dx:Double,dy:Double){val c=mutable.value.view.camera;val points=core.mapPoints(c,true,listOf(Point(0.0,0.0),Point(dx,dy)));update{it.copy(view=it.view.copy(camera=c.copy(center=Point(c.center.x-points[1].x+points[0].x,c.center.y-points[1].y+points[0].y))))}}
    fun select(id:String,extend:Boolean=false){val item=mutable.value.document?.render?.items?.firstOrNull{it.objectId==id}?:return;if(item.locked)return;cancelWidth();update{it.copy(selected=if(extend)it.selected.toggle(id)else setOf(id),color=item.style.rgba,width=item.style.width.coerceIn(.25,256.0))};instructionFocus.localSelection(mutable.value.selected.singleOrNull());if(item.shape is Shape.Marker)instructionEditor.focusObject(id)}
    fun selectAll(){instructionFocus.localSelection(null);update{it.copy(selected=it.document?.render?.items?.filterNot{item->item.locked}?.map{item->item.objectId}?.toSet()?:emptySet())}}
    fun clearSelection(){instructionFocus.localSelection(null);cancelInput();update{it.copy(selected=emptySet(),preview=emptyMap(),draft=null,textAnchor=null)}}
    fun pointerDown(viewPoint:Point,extend:Boolean,panButton:Boolean,uptimeMs:Long=System.nanoTime()/1_000_000,pressure:Float=1f,semanticModifier:Boolean=false){if(ai.state.value.open||(!panButton&&mutable.value.tool!=Tool.Pan&&!aiResultReady()))return;val s=mutable.value;if(s.busy||instructionEditor.state.value.busy||semanticEditor.state.value.busy||s.document==null||s.background==null||(!s.canvasReady&&!panButton&&s.tool!=Tool.Pan)||s.renderIssue!=null)return;val binding=s.document.binding(generation);val point=core.mapPoints(s.view.camera,true,listOf(viewPoint)).single();val pan=panButton||(s.tool==Tool.Pan&&!selections.state.value.active&&!erasers.state.value.active)
        if(!pan)when(erasers.tools.route()){
            EraserInputRoute.Selection->{selectionPointer=selections.interaction.begin(viewPoint,s.view.camera);return}
            EraserInputRoute.Eraser->{eraserPointer=erasers.interaction.begin(viewPoint,s.view.camera);return}
            EraserInputRoute.Blocked->return
            EraserInputRoute.Editor->Unit
        }
        if(selections.state.value.busy||erasers.state.value.busy)return
        if(s.tool==Tool.Callout&&!pan){markerPress=MarkerPress(binding,point,s.view.camera,semanticModifier);update{it.copy(draft=point to point)};return}
        if(s.tool==Tool.Pen&&!pan){beginInk(DesktopSample(point,uptimeMs,pressure),binding);return}
        if(s.tool==Tool.Text&&!pan){update{it.copy(textAnchor=point)};return}
        if(s.tool !in listOf(Tool.Select,Tool.Pan)&&!pan){update{it.copy(draft=point to point)};drag=Drag(point,s.view,emptyList(),null,false,false,binding,newObject=newObjectPreview(s,binding),tool=s.tool);return}
        var selected=s.document?.render?.items?.filter{it.objectId in s.selected}?:emptyList();val bounds=union(selected.map{it.bounds});val tolerance=10.0/s.view.camera.scale
        val resize=!pan&&bounds!=null&&kotlin.math.abs(point.x-bounds.x-bounds.width)<=tolerance&&kotlin.math.abs(point.y-bounds.y-bounds.height)<=tolerance
        if(!pan&&!resize){val item=s.document?.render?.items?.asReversed()?.firstOrNull{!it.locked&&point.x>=it.bounds.x&&point.x<=it.bounds.x+it.bounds.width&&point.y>=it.bounds.y&&point.y<=it.bounds.y+it.bounds.height};if(item!=null){if(item.objectId !in s.selected||extend)select(item.objectId,extend)}else if(!extend)clearSelection();selected=mutable.value.document?.render?.items?.filter{it.objectId in mutable.value.selected}?:emptyList()}
        if(!pan&&selected.any{it.shape is Shape.Guide}){report("Use selection tools to edit pixel coverage; guides keep their source coordinates.");return}
        drag=Drag(point,s.view,selected,union(selected.map{it.bounds}),resize,pan,binding,if(!pan)previewFor(selected,binding,PreviewKind.HandleDrag)else null)
    }
    fun pointerMove(viewPoint:Point,uptimeMs:Long=System.nanoTime()/1_000_000,pressure:Float=1f){
        markerPress?.let { press ->
            if (!geometryMatches(press.binding, mutable.value)) { cancelDrag(); return }
            press.end = core.mapPoints(press.camera, true, listOf(viewPoint)).single()
            update { it.copy(draft = press.first to press.end) }; return
        }
        if(eraserPointer){erasers.interaction.append(viewPoint);return};if(selectionPointer){selections.interaction.append(viewPoint);return};ink?.let{run->val point=core.mapPoints(run.camera,true,listOf(viewPoint)).single();if(!run.offer(DesktopSample(point,uptimeMs,pressure))){cancelInk();report("The input queue or stroke limit was reached. The unfinished stroke was cancelled.")};return};val d=drag?:return;if(!geometryMatches(d.binding,mutable.value)){cancelDrag();return};val point=core.mapPoints(d.initial.camera,true,listOf(viewPoint)).single();val dx=point.x-d.start.x;val dy=point.y-d.start.y
        if(d.pan){update{it.copy(view=it.view.copy(camera=d.initial.camera.copy(center=Point(d.initial.camera.center.x-dx,d.initial.camera.center.y-dy))))};return}
        if(mutable.value.draft!=null){update{it.copy(draft=d.start to point)};draftShape(d.tool,d.start,point)?.let{d.newObject?.update(it)};return}
        val b=d.bounds;val preview=d.selected.associate{item->item.objectId to if(d.resize&&b!=null&&b.width>0&&b.height>0)scaledAbout(item.transform,Point(b.x,b.y),((b.width+dx)/b.width).coerceIn(0.01,100.0),((b.height+dy)/b.height).coerceIn(0.01,100.0))else translated(item.transform,dx,dy)}
        update{it.copy(preview=preview)}
        d.selected.singleOrNull()?.let{item->d.live?.update(preview[item.objectId]?:item.transform,item.style)}
    }
    fun pointerUp(viewPoint:Point?=null,uptimeMs:Long=System.nanoTime()/1_000_000,pressure:Float=1f){
        markerPress?.let { press ->
            val binding = press.binding; val point = press.first
            markerPress = null; update { it.copy(draft = null) }
            val current = mutable.value
            if (geometryMatches(binding, current)) {
                val end = viewPoint?.let { core.mapPoints(press.camera, true, listOf(it)).single() } ?: press.end
                val matrix = core.cameraMatrix(press.camera)
                semanticEditor.place(point, ObjectStyle(current.color, current.width), matrix, semanticDropBox(point, end, matrix), press.invertSnapping)
            }
            else report("The document changed. Place the marker again on the current revision.")
            return
        }
        if(eraserPointer){eraserPointer=false;erasers.interaction.finish(viewPoint);return};if(selectionPointer){selectionPointer=false;selections.interaction.finish(viewPoint);return};if(ink!=null){if(viewPoint!=null)pointerMove(viewPoint,uptimeMs,pressure);finishInk();return};val s=mutable.value;val d=drag;drag=null;if(d==null)return;if(!geometryMatches(d.binding,s)){val old=d.live?:d.newObject;live.cancelLater(old?.attachment,old?.id);cancelDrag();return}
        if(s.draft!=null){val(a,b)=s.draft;val r=Rect(minOf(a.x,b.x),minOf(a.y,b.y),kotlin.math.abs(a.x-b.x),kotlin.math.abs(a.y-b.y));val shape=when(d.tool){Tool.Rectangle->Shape.Rectangle(r);Tool.Ellipse->Shape.Ellipse(r);Tool.Line->Shape.Line(listOf(a,b));Tool.Arrow->Shape.Arrow(listOf(a,b));else->null};update{it.copy(draft=null)};val provisional=d.newObject;val geometry=draftShape(d.tool,a,b);if(provisional!=null){if(geometry!=null)gestureEdit(listOf(EditCommand.Create(provisional.objectId,provisional.layer,geometry,provisional.style)),d.binding,provisional)else live.cancelLater(provisional.attachment,provisional.id)}else if(shape!=null&&r.width+r.height>0.01)create(shape,d.binding);return}
        val edits=s.preview.filter{(id,transform)->d.selected.any{it.objectId==id&&it.transform!=transform}}.map{EditCommand.SetTransform(it.key,it.value)};update{it.copy(preview=emptyMap())};gestureEdit(edits,d.binding,d.live)
    }
    fun cancelDrag(){markerPress=null;eraserPointer=false;erasers.interaction.cancelGesture();selectionPointer=false;selections.interaction.cancelGesture();val old=drag?.let{it.live?:it.newObject};drag=null;live.cancelLater(old?.attachment,old?.id);update{it.copy(preview=emptyMap(),draft=null)}}
    private fun newObjectPreview(state:EditorState,binding:RenderBinding):NewObjectPreviewLease?{
        val attachment=live.attachment()?:return null;val shape=draftTemplate(state.tool)?:return null
        val layer=state.document?.layers?.firstOrNull{it.visible&&!it.locked&&it.blend=="normal"}?:return null;val now=System.currentTimeMillis()
        return NewObjectPreviewLease(core.newId(now.toULong()),attachment,core.newId(now.toULong()),binding.documentId,layer.id,now,shape,ObjectStyle(state.color,state.width))
    }
    private fun previewFor(items:List<RenderItem>,binding:RenderBinding,kind:PreviewKind):ObjectPreviewLease?{
        val item=items.singleOrNull()?:return null;val attached=live.attachment()?:return null
        return ObjectPreviewLease(core.newId(System.currentTimeMillis().toULong()),attached,binding.documentId,item.objectId,kind)
    }
    private fun cancelWidth(){val old=widthGesture;widthGesture=null;live.cancelLater(old?.live?.attachment,old?.live?.id);update{it.copy(previewStyles=emptyMap())}}
    fun cancelInput(){cancelDrag();cancelWidth();cancelInk()}
    private fun beginInk(first:DesktopSample,binding:RenderBinding){
        val s=mutable.value;val p=project?:return;val doc=s.document?:return;val layer=doc.layers.firstOrNull{it.visible&&!it.locked&&it.blend=="normal"}?:return
        val now=System.currentTimeMillis();val info=doc.render.revision
        val options=StrokeOptions(core.newId(now.toULong()),core.newId(now.toULong()),core.newId(now.toULong()),binding.documentId,layer.id,info.deviceId,info.nextLamport,now,"pen",s.width,s.color)
        val run=DesktopInkRun(options,binding,s.view.camera,layer,live.attachment(),first);ink=run
        val job=task(binding.epoch){try{
            run.execute(p,{item->if(project===p&&generation==binding.epoch)update{it.copy(wetStroke=item)}},
                {wetCommitted=options.objectId;refresh()},
                {cancel->live.finish(run.attachment,options.gestureId,cancel)})
        }finally{if(ink===run)ink=null;if(inkWork?.first===run)inkWork=null;if(!run.committed)update{it.copy(wetStroke=null)}}}
        if(job==null){run.cancel();ink=null}else inkWork=run to job
    }
    private fun finishInk(){val current=ink;ink=null;current?.finish()}
    private fun cancelInk(){val current=inkWork?.first?:ink;ink=null;try{current?.cancel()}catch(error:CoreFailure){report("Core ${error.kind.name.lowercase()} while cancelling ink.")};if(current?.committed!=true){wetCommitted=null;update{it.copy(wetStroke=null)}}}
    internal fun create(shape:Shape,binding:RenderBinding?=mutable.value.document?.binding(generation),afterSuccess:()->Unit={}){val s=mutable.value;if(s.renderIssue!=null)return;val layer=s.document?.layers?.firstOrNull{it.visible&&!it.locked&&it.blend=="normal"}?:return;edit(listOf(EditCommand.Create(core.newId(System.currentTimeMillis().toULong()),layer.id,shape,ObjectStyle(s.color,s.width))),binding,afterSuccess)}
    fun acceptText(text:String){val anchor=mutable.value.textAnchor?:return;if(text.isNotBlank()&&!mutable.value.busy&&queued==0)create(Shape.Text(anchor,text,"Inter",18.0),afterSuccess={update{it.copy(textAnchor=null)}})}
    fun delete(){
        val state = mutable.value
        val markers = state.document?.render?.items.orEmpty().filter { it.objectId in state.selected && it.shape is Shape.Marker }
        if (markers.isNotEmpty()) {
            if (markers.size == 1 && state.selected.size == 1) instructionEditor.deleteMarker(markers.single().objectId)
            else report("Delete numbered markers individually in Instructions so their linked instruction and numbering remain atomic.")
        } else edit(state.selected.map{EditCommand.Delete(it)})
    }
    fun undo(redo:Boolean){task(generation){cancelDrag();project?.undoRedo(options(),redo)?:error("No project");refresh()}}
    fun nudge(dx:Double,dy:Double){editCurrent{document,selected->document.render.items.filter{it.objectId in selected&&!it.locked}.map{EditCommand.SetTransform(it.objectId,translated(it.transform,dx,dy))}}}
    fun color(value:UInt){update{it.copy(color=value)};editCurrent{document,selected->document.render.items.filter{it.objectId in selected&&!it.locked}.map{EditCommand.SetStyle(it.objectId,it.style.copy(rgba=value))}}}
    fun width(value:Double,commit:Boolean){
        val width=value.coerceIn(0.25,256.0);update{it.copy(width=width)}
        val s=mutable.value;val doc=s.document?:return
        if(!commit){
            if(s.busy||!s.canvasReady)return
            if(widthGesture==null){val items=doc.render.items.filter{it.objectId in s.selected&&!it.locked};val bound=doc.binding(generation);widthGesture=WidthGesture(bound,items,previewFor(items,bound,PreviewKind.Slider))}
            val gesture=widthGesture?:return
            if(!geometryMatches(gesture.binding,s)){cancelWidth();return}
            update{it.copy(previewStyles=gesture.items.associate{item->item.objectId to item.style.copy(width=width)})}
            gesture.items.singleOrNull()?.let{gesture.live?.update(it.transform,it.style.copy(width=width))}
        }else{
            val gesture=widthGesture;widthGesture=null;update{it.copy(previewStyles=emptyMap())}
            if(gesture!=null)gestureEdit(gesture.items.filter{it.style.width!=width}.map{EditCommand.SetStyle(it.objectId,it.style.copy(width=width))},gesture.binding,gesture.live)
            else editCurrent{document,selected->document.render.items.filter{it.objectId in selected&&!it.locked}.map{EditCommand.SetStyle(it.objectId,it.style.copy(width=width))}}
        }
    }
    fun commitWidth(){width(mutable.value.width,true)}
    fun cycleMarker(){val ids=mutable.value.document?.render?.items?.filter{it.shape is Shape.Marker}?.sortedBy{(it.shape as Shape.Marker).number}?.map{it.objectId}.orEmpty();if(ids.isEmpty()){report("No markers in this document.");return};val current=ids.indexOf(mutable.value.selected.firstOrNull());select(ids[(current+1)%ids.size])}
    fun follow(){if(mutable.value.view.peer==null){report("Peer view is unavailable until an authenticated session is connected.");return};update{it.copy(view=it.view.copy(followPeer=!it.view.followPeer))}}
    fun matchPeer(){if(mutable.value.view.peer==null)report("No authenticated peer viewport is available.")else update{it.copy(view=it.view.matchPeer())}}
    fun outline(){update{it.copy(view=it.view.copy(showPeerOutline=!it.view.showPeerOutline))}}
    private fun sessionStatus(value:SessionStatus?){update{s->
        val peer=value?.peerViewport?.takeIf{it.documentId==s.document?.documentId}
        val camera=peer?.let{peerCamera(it,s.view.camera)}
        s.copy(sync=value,peerViewport=peer,sessionRole=s.sessionRole?.let{role->if(value!=null)role.copy(pending=value.pending,blocked=value.blocked)else role},view=if(camera!=null)s.view.receivePeerView(camera)else s.view.copy(peer=null))
    }}
    fun shareViewport(value:Boolean){update{it.copy(shareViewport=value)};if(!value){viewportJob?.cancel();viewportJob=null}}
    private fun scheduleViewport(){if(viewportJob?.isActive==true)return;viewportJob=scope.launch{delay(34);val state=mutable.value;val doc=state.document;if(state.shareViewport&&doc!=null){val camera=state.view.camera;live.sendViewport(doc.documentId,core.mapPoints(camera,true,listOf(Point(0.0,0.0),Point(camera.viewportWidth,0.0),Point(camera.viewportWidth,camera.viewportHeight),Point(0.0,camera.viewportHeight))))}}}
    fun inspectSessionRole(sessions:DesktopSessionController){sessionProvider=sessions;task(generation){val p=project?:return@task;val role=sessions.requireService().projectRole(p);if(project===p)update{it.copy(sessionRole=role)}}}
    fun connectSession(sessions:DesktopSessionController,peer:String,endpoints:List<SessionEndpoint>,hosting:Boolean,tetherInterface:UInt?){
        val chosen=endpoints.map{SessionEndpoint(it.carrier,it.address)}.distinct().sortedBy{it.carrier.ordinal}
        if(sessionWork?.isActive==true){report("A connection request is already running. Finish or cancel it first.");return};sessionProvider=sessions
        val epoch=generation;cancelInput()
        sessionWork=task(epoch){val p=project?:throw HandoffFailure("Open a project first, or receive a project from its host.");val service=sessions.requireService();val role=service.projectRole(p)
            if(role.isHost!=hosting)throw HandoffFailure(if(role.isHost)"This device owns the project's journal. Choose Share project." else "This is a replica. Connect to its original host.")
            if(!hosting&&peer!=role.hostDeviceId)throw HandoffFailure("Select the original host paired with this replica.")
            require(chosen.isNotEmpty()&&chosen.size<=8);chosen.forEach{checkedEndpoint(it.address)}
            if(chosen.any{it.carrier==SessionCarrier.QuicTether}&&tetherInterface==null)throw HandoffFailure("Choose the tether interface before connecting so its default-route safety can be checked.")
            live.close();update{it.copy(sessionRole=role,routeWarnings=emptyList())}
            val link=if(hosting)service.host(p,peer,chosen)else service.connect(p,peer,chosen)
            var adopted=false
            try{if(project!==p||generation!=epoch)throw CancellationException("Project changed");live.install(link){if(tetherInterface!=null){val findings=service.tetherRouteWarnings(tetherInterface);if(project===p&&generation==epoch)update{it.copy(routeWarnings=findings)}}};adopted=true;instructionFocus.attach(link);scheduleViewport()}
            finally{if(!adopted)withContext(NonCancellable){link.close()}}
        }
    }
    fun receiveProject(sessions:DesktopSessionController,peer:String,endpoints:List<SessionEndpoint>,expectedProject:String?,tetherInterface:UInt?=null){
        if(sessionWork?.isActive==true){report("A connection request is already running. Finish or cancel it first.");return};sessionProvider=sessions
        val chosen=endpoints.map{SessionEndpoint(it.carrier,it.address)}.distinct().sortedBy{it.carrier.ordinal};val epoch=generation
        sessionWork=task(epoch){require(chosen.isNotEmpty()&&chosen.size<=8);chosen.forEach{checkedEndpoint(it.address)};val service=sessions.requireService();if(chosen.any{it.carrier==SessionCarrier.QuicTether}){if(tetherInterface==null)throw HandoffFailure("Choose the tether interface before receiving a project.");val findings=service.tetherRouteWarnings(tetherInterface);update{it.copy(routeWarnings=findings)}};val destination=location.path.toAbsolutePath().resolve("${core.newId(System.currentTimeMillis().toULong())}.vwb");withContext(Dispatchers.IO){Files.createDirectories(location.path)};val received=service.receiveProject(destination.toString(),peer,chosen,expectedProject?.trim()?.takeIf{it.isNotEmpty()});attach(received,destination);val role=service.projectRole(received);update{it.copy(sessionRole=role,message="Project received and saved locally. Connect the replica to continue live editing.")}}
    }
    fun disconnectSession(){cancelInput();task(generation){live.close();update{it.copy(routeWarnings=emptyList())}}}
    fun cancelSessionWork(){sessionWork?.cancel()}
    suspend fun disconnectPeer(peer:String){if(mutable.value.sync?.peerDeviceId!=peer)return;cancelInput();edits.withLock{if(mutable.value.sync?.peerDeviceId==peer)live.close()}}
    fun showExport(){if(mutable.value.document==null){report("Open a document before exporting.");return};update{it.copy(exportOpen=true)}}
    fun dismissExport(){update{it.copy(exportOpen=false)}}
    fun exportSettings(settings:ExportSettings){update{it.copy(exportSettings=settings.copy(quality=settings.quality.coerceIn(1,100)),message=null)}}
    fun exportRequest():ExportRequest?=try{captureExport(mutable.value,core)}catch(error:HandoffFailure){report(error.message?:"Export region unavailable.");null}
    private fun requireCurrent(request:ExportRequest){if(!request.matches(mutable.value))throw HandoffFailure("The project, revision, region or export settings changed. Review them and prepare the export again.")}
    private fun transferApi():WorkbenchFileTransfers=project as? WorkbenchFileTransfers?:throw HandoffFailure("File export is unavailable in this core binding.")
    private fun fileOptions(request:ExportRequest,workspace:TransferWorkspace)=FileExportOptions(request.options(),workspace.output.toString(),workspace.directory.toString())
    fun checkExport(request:ExportRequest){transfer(request.epoch){requireCurrent(request);val workspace=transferFiles.create();try{val result=transferApi().preflightFile(fileOptions(request,workspace));if(!request.matches(result.revision))throw HandoffFailure("The project changed during preflight. Check it again.");requireCurrent(request);update{it.copy(exportCheck=ExportCheck(request,result),message="Format preflight passed. Marked geometry and result assets are checked again during export.")}}finally{workspace.close()}}}
    private suspend fun prepareExport(request:ExportRequest):CachedExport{
        requireCurrent(request);val p=project?:throw HandoffFailure("Open a project before exporting.")
        if(!request.matches(p.info())){refresh();throw HandoffFailure("The core revision changed. Review it before exporting.")}
        cachedExport?.takeIf{it.receipt.request==request}?.let{update{state->state.copy(exportReceipt=it.receipt)};return it}
        discardExport();val workspace=transferFiles.create();var retained=false
        try{val api=transferApi();val options=fileOptions(request,workspace);val preflight=api.preflightFile(options);if(!request.matches(preflight.revision))throw HandoffFailure("The project changed during preflight.");requireCurrent(request);update{it.copy(exportCheck=ExportCheck(request,preflight))};val result=api.exportTransfer(options);val completed=completeExport(request,result);requireCurrent(request);if(withContext(Dispatchers.IO){Files.size(workspace.output).toULong()}!=completed.bytes)throw HandoffFailure("The completed export size differs from its receipt.");val cached=CachedExport(workspace,completed);cachedExport=cached;retained=true;update{it.copy(exportReceipt=completed)};return cached}finally{if(!retained)workspace.close()}
    }
    fun prepareExportFile(request:ExportRequest){transfer(request.epoch){val cached=prepareExport(request);report("${cached.receipt.revision} prepared at ${cached.receipt.width} × ${cached.receipt.height} px.")}}
    fun saveExport(request:ExportRequest,path:Path){val destination=path.toAbsolutePath().normalize();transfer(request.epoch){val cached=prepareExport(request);publishExport(cached.workspace.output,destination,cached.receipt.bytes){request.matches(mutable.value)};report("${cached.receipt.revision} saved at ${cached.receipt.width} × ${cached.receipt.height} px.")}}
    fun copyExport(request:ExportRequest){transfer(request.epoch){if(!request.settings.encoding.png)throw HandoffFailure("Clipboard copy requires a PNG export. Choose PNG 8 or 16 bit.");val cached=prepareExport(request);requireCurrent(request);handoff.copy(cached.workspace.output,cached.receipt);report("${cached.receipt.revision} copied as PNG and DIBV5.")}}
    fun prepareDragExport(request:ExportRequest){transfer(request.epoch){if(!request.settings.encoding.png)throw HandoffFailure("File drag requires PNG. Choose PNG 8 or 16 bit.");val cached=prepareExport(request);requireCurrent(request);val lease=handoff.stage(cached.workspace.output,cached.receipt);var adopted=false;try{requireCurrent(request);preparedDrag?.second?.close();preparedDrag=cached.receipt to lease;adopted=true;update{it.copy(dragPrepared=true,exportOpen=false,message="PNG drag prepared. Drag the button to the destination; nothing is sent automatically.")}}finally{if(!adopted)lease.close()}}}
    internal fun takePreparedDrag():DesktopDragFile?{val offer=preparedDrag?:return null;if(!offer.first.request.matches(mutable.value)){offer.second.close();preparedDrag=null;update{it.copy(dragPrepared=false)};return null};preparedDrag=null;update{it.copy(dragPrepared=false)};return offer.second}
    fun cancelTransfer(){transferJob?.cancel();report("Cancelling transfer; waiting for native work to release its files.")}
}
private fun <T> Set<T>.toggle(value:T):Set<T> = if(value in this)this-value else this+value
private fun bitmap(value:BackgroundImage):ImageBitmap {
    val width=value.width.toInt();val height=value.height.toInt();val image=BufferedImage(width,height,BufferedImage.TYPE_INT_ARGB);val row=IntArray(width)
    for(y in 0 until height){for(x in 0 until width){val i=(y*width+x)*4;row[x]=((value.rgba[i+3].toInt() and 255) shl 24) or ((value.rgba[i].toInt() and 255) shl 16) or ((value.rgba[i+1].toInt() and 255) shl 8) or (value.rgba[i+2].toInt() and 255)};image.setRGB(0,y,width,1,row,0,width)}
    val bytes=ByteArrayOutputStream().use{check(ImageIO.write(image,"png",it));it.toByteArray()};return Image.makeFromEncoded(bytes).toComposeImageBitmap()
}
