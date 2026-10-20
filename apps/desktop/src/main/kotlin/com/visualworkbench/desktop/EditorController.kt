package com.visualworkbench.desktop

import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import com.visualworkbench.shared.*
import com.visualworkbench.desktop.precision.precisionResizeHit
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
    val wetStroke:RenderItem?=null,val shareViewport:Boolean=false,val sessionRole:ProjectSessionRole?=null,
    val backgroundIssue:String?=null,val backgroundFailureKind:CoreFailureKind?=null,val originalFormatNotice:String?=null,
    val originalFormatInfo:OriginalFormatInfo?=null,val originalFormatUnavailable:Boolean=false,
    val composedBackground:RenderBinding?=null,val composedGuideIds:Set<String> = emptySet(),
    val documentFrame:DocumentDrawFrame<AutoCloseable>?=null,val documentPage:UInt=0u,val precision:PrecisionFrame=PrecisionFrame(),
    val giantOwnerVersion:Long=0L,val giantActive:Boolean=false,val giantReady:Boolean=false,val giantIssue:String?=null,val giantViewport:GiantViewport?=null,
    val isStreamDraft:Boolean=false) {
    val exportReady:Boolean get()=exportReceipt?.request?.matches(this)==true
}
internal fun composedSelectionGuideIds(document:DocumentSnapshot,controls:DocumentControlSnapshot,
    binding:WorkflowBinding):Set<String>{
    if(controls.binding!=binding)throw WorkflowFailure(WorkflowFailureKind.Stale)
    val kinds=controls.objects.associate{it.objectId to it.kind}
    return document.render.items.filter{it.shape is Shape.Polygon}.mapNotNull{item->
        when(kinds[item.objectId]){
            "polygon"->null
            "vector selection"->item.objectId
            else->throw WorkflowFailure(WorkflowFailureKind.Stale)
        }
    }.toSet()
}
internal data class GiantDisplayChoice(val binding:WorkflowBinding,val sourceAssetId:String,
    val content:GiantTileContent,val assumeUntaggedSrgb:Boolean)
internal fun expectedGiantContent(document:DocumentSnapshot):GiantTileContent =
    if(document.hasToneAdjustments)GiantTileContent.Composed else GiantTileContent.Source
internal fun giantDisplayChoice(document:DocumentSnapshot,info:OriginalFormatInfo?,approvedSourceId:String?):GiantDisplayChoice? {
    val revision=document.render.revision
    val binding=WorkflowBinding(revision.projectId,document.documentId,revision.hostSeq,revision.stateHash)
    if(info==null||info.binding!=binding||info.sourceAssetId.length!=64)return null
    if(info.untagged&&approvedSourceId!=info.sourceAssetId)return null
    return GiantDisplayChoice(binding,info.sourceAssetId,expectedGiantContent(document),info.untagged)
}
internal fun <T:Any> giantChoiceMatches(frame:GiantVisibleFrame<T>,choice:GiantDisplayChoice,
    epoch:Long,viewport:GiantViewport):Boolean = frame.content==choice.content&&
    frame.assetId==choice.sourceAssetId&&frame.matches(choice.binding,epoch,viewport)
internal fun giantChoiceRequiresRetirement(prior:GiantDisplayChoice?,next:GiantDisplayChoice?,forceReplace:Boolean=false):Boolean =
    forceReplace||prior!=next||next==null
internal suspend fun retryWithFreshOriginalFormat(refresh:suspend ()->Unit,load:suspend ()->Unit){
    refresh()
    currentCoroutineContext().ensureActive()
    load()
}
private data class Drag(val start:Point,val initial:ViewState,val selected:List<RenderItem>,val bounds:Rect?,val resize:Boolean,val pan:Boolean,val binding:RenderBinding,val live:ObjectPreviewLease?=null,val newObject:NewObjectPreviewLease?=null,val tool:Tool=Tool.Select)
private data class WidthGesture(val binding:RenderBinding,val items:List<RenderItem>,val live:ObjectPreviewLease?)
private data class CachedExport(val workspace:TransferWorkspace,val receipt:CompletedExport)
class EditorController internal constructor(private val scope:CoroutineScope,val core:WorkbenchCore,private val handoff:DesktopHandoff,val location:LocationPolicy,private val deviceId:String,workRoot:Path=location.path.resolve(".handoff"),private val streamDraftOnly:Boolean=false) {
    constructor(scope:CoroutineScope,core:WorkbenchCore,host:HostService,location:LocationPolicy,deviceId:String,workRoot:Path):this(scope,core,NativeDesktopHandoff(host),location,deviceId,workRoot)
    private val mutable=MutableStateFlow(EditorState());val state:StateFlow<EditorState> = mutable
    private var precisionClosed=false
    private fun cancelPrecision(){if(!precisionClosed)precision.cancel()}
    private fun closePrecision(){precision.close();precisionClosed=true}
    private var panningView=false
    private var synchronizingPrecision=false
    val precision:PrecisionController=PrecisionController(core,::precisionChanged)
    private fun precisionChanged(){if(!closing&&!synchronizingPrecision)update{it.copy(precision=precision.frame())}}
    private var precisionContact=false
    private var documentPress:Pair<Point,Point>?=null
    private var documentPressRegion=false
    private var documentPressBinding:DocumentFrameBinding?=null
    private var giantMode=false
    private var giantChoice:GiantDisplayChoice?=null
    private var approvedGiantSourceId:String?=null
    private var updatingGiant=false
    private val giantDraw=java.util.concurrent.atomic.AtomicReference<GiantVisibleFrame<GiantBitmap>?>(null)
    private val giantOwner=GiantAttachmentOwner<GiantDesktopView>(
        attachment={val p=project;val doc=mutable.value.document;if(p!=null&&doc!=null&&!closing)DocumentAttachment(p,doc,generation)else null},
        create={image,resources,binding,epoch,current->
            val choice=checkNotNull(giantChoice).also{check(it.binding==binding)}
            GiantDesktopView.create(scope,image,resources,binding,epoch,
                {value,attached->current(value,attached)&&giantChoice==choice},::cancelInput,
                {value->if(giantChoice==choice)update{it.copy(giantReady=value.ready,giantIssue=value.message)}},
                choice.content,choice.assumeUntaggedSrgb)
        },closeView={it.close()},changed={giantDraw.set(null);update{it.copy(giantReady=false,giantOwnerVersion=it.giantOwnerVersion+1L)}},
        diskCacheBytes=4uL*1024uL*1024uL*1024uL,ramCacheBytes=128uL*1024uL*1024uL,
        failedCreationOwner={ (it as? GiantDesktopCreationFailure)?.owner })
    internal val giantView:GiantDesktopView? get()=giantOwner.current
    internal fun giantFrameCurrent(frame:GiantVisibleFrame<GiantBitmap>,drawState:EditorState?=null):Boolean {
        val state=mutable.value;val document=state.document?:return false
        val revision=document.render.revision
        val choice=giantChoice?:return false
        val viewport=state.giantViewport?:return false
        return giantDraw.get()===frame&&state.giantActive&&choice.content==expectedGiantContent(document)&&
            giantDisplayChoice(document,state.originalFormatInfo,approvedGiantSourceId)==choice&&
            (drawState==null||(drawState.document===document&&drawState.view.camera==state.view.camera&&drawState.precision==state.precision))&&
            choice.binding==WorkflowBinding(revision.projectId,document.documentId,revision.hostSeq,revision.stateHash)&&
            giantChoiceMatches(frame,choice,generation,viewport)&&
            giantOwner.current?.matchingFrame(choice.binding,generation,viewport)===frame
    }
    private fun giantInputReady():Boolean=giantDraw.get()?.let{giantFrameCurrent(it)}==true
    private fun composedInputReady(state:EditorState):Boolean =
        state.composedBackground==state.document?.binding(generation)||
            giantDraw.get()?.let{it.content==GiantTileContent.Composed&&giantFrameCurrent(it)}==true
    private suspend fun detachGiant(){giantMode=false;giantChoice=null;giantDraw.set(null);giantOwner.detach()}
    private suspend fun loadGiantBackground(p:WorkbenchProject,doc:DocumentSnapshot,forceReplace:Boolean){
        val epoch=generation;val ticket=refreshTicket
        val info=mutable.value.originalFormatInfo
        val choice=giantDisplayChoice(doc,info,approvedGiantSourceId)
        val unapproved=info?.untagged==true&&choice==null
        giantMode=true;giantDraw.set(null)
        if(giantChoiceRequiresRetirement(giantChoice,choice,forceReplace)){
            giantChoice=null;giantOwner.invalidate()
            try{giantOwner.detach()}catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){update{it.copy(giantActive=true,giantReady=false,giantIssue="The previous tiled renderer could not retire. Its resources remain reserved; reopen after recovery.")};return}
        }
        fun current()=!closing&&project===p&&generation==epoch&&refreshTicket==ticket&&mutable.value.document===doc
        if(!current())return
        update{it.copy(background=null,composedBackground=null,composedGuideIds=emptySet(),giantActive=true,
            giantReady=false,giantIssue=null,needsColorConsent=unapproved,
            backgroundIssue=when{unapproved->"Tiled display requires explicit sRGB consent for this untagged source. The original remains unchanged."
                choice==null->"The giant source identity could not be verified. Reopen or retry display.";else->null})}
        if(choice==null)return
        try{
            val guides=if(choice.content==GiantTileContent.Composed&&doc.render.items.any{it.shape is Shape.Polygon})
                composedSelectionGuideIds(doc,documentControlWorkflows(p).snapshot(doc.documentId),choice.binding) else emptySet()
            if(!current())return
            giantChoice=choice
            update{it.copy(composedGuideIds=guides)}
            giantOwner.bind(DocumentAttachment(p,doc,epoch))?.update(core,mutable.value.view.camera,mutable.value.precision)
        }
        catch(cancel:CancellationException){throw cancel}
        catch(error:Exception){if(current())update{it.copy(giantIssue=if(error is GiantImageFailure&&
            error.kind==GiantImageFailureKind.Unsupported&&choice.content==GiantTileContent.Composed)
            "Committed giant-image composition is unavailable for this source. The original and saved edits are preserved."
            else "The original is preserved. Its verified tiled display is unavailable. Retry display or reopen the project.")}}
    }
    private fun syncGiant(){
        if(!giantMode){giantDraw.set(null);return}
        val state=mutable.value;val doc=state.document?:run{giantDraw.set(null);return}
        val choice=giantChoice
        if(choice==null||giantDisplayChoice(doc,state.originalFormatInfo,approvedGiantSourceId)!=choice){giantDraw.set(null);return}
        val view=giantOwner.current?:run{giantDraw.set(null);return}
        if(!updatingGiant){updatingGiant=true;try{view.update(core,mutable.value.view.camera,mutable.value.precision)}
            catch(_:Exception){giantDraw.set(null);mutable.value=mutable.value.copy(giantReady=false,giantIssue="This tiled viewport exceeds its display allowance. Reduce the window or retry display.");return}
            finally{updatingGiant=false}}
        val present=mutable.value
        val presentDoc=present.document
        if(presentDoc!==doc||giantChoice!=choice||giantDisplayChoice(doc,present.originalFormatInfo,approvedGiantSourceId)!=choice){giantDraw.set(null);return}
        val revision=doc.render.revision
        giantDraw.set(present.giantViewport?.let{viewport->if(choice.binding==WorkflowBinding(revision.projectId,doc.documentId,revision.hostSeq,revision.stateHash))
            view.matchingFrame(choice.binding,generation,viewport)?.takeIf{giantChoiceMatches(it,choice,generation,viewport)} else null})
    }
    private val documentOwner:EditorDocuments=EditorDocuments(scope,core,
        { val p=project;val snapshot=mutable.value.document;if(p!=null&&snapshot!=null&&!closing&&!streamDraftOnly)DocumentAttachment(p,snapshot,generation)else null },
        { val s=mutable.value;!closing&&!streamDraftOnly&&!streamEditsSealed&&!workflows.blocksInput()&&!frames.state.value.blocksInput&&!s.busy&&queued==0&&!s.exportOpen&&!s.pasteOpen&&!ai.state.value.open&&!instructionEditor.state.value.busy&&!semanticEditor.state.value.busy&&!instructionEditor.hasUnsaved()&&instructionAdmission.allows(s.textAnchor!=null,s.exportOpen,s.pasteOpen)&&drag==null&&ink==null&&markerPress==null&&documentPress==null&&!eraserPointer&&!selectionPointer },
        { attached,page -> edits.withLock {
            if(project!==attached.project||generation!=attached.epoch||mutable.value.document?.render?.revision?.sameDocumentRevision(attached.snapshot.render.revision)!=true)throw DocumentFailure(DocumentFailureKind.Stale)
            refreshTicket++;documentRefresh.invalidate()
            cancelInput();frames.beginDeparture();frames.detach();workflows.detach();rotation.detach();detachGiant();erasers.detach();selections.detach();documentOwnerDiscardDraw()
            val next=(attached.project as WorkbenchDocumentsProject).selectDocumentPage(attached.snapshot.documentId,page)
            if(closing||project!==attached.project||generation!=attached.epoch)throw DocumentFailure(DocumentFailureKind.Stale)
            update{it.copy(document=next,documentPage=page,background=null,canvasReady=false,selected=emptySet())}
            selections.bind(attached.project,next,generation,changeSequence,mutable.value.view.camera)
            erasers.bind(attached.project,next,generation,changeSequence,mutable.value.view.camera)
            instructionEditor.refresh();instructionFocus.revisionChanged();semanticEditor.refresh();fit();bindWorkflows();enterWorkflows()
        } },{update{it.copy(documentFrame=it.document?.let{snapshot->documentOwnerFrame(snapshot)})}},location.path)
    val documents:DocumentController get()=documentOwner.controller
    internal fun documentReader(expected:DocumentDrawFrame<AutoCloseable>?):DocumentDrawLease<com.visualworkbench.desktop.documents.PreparedDocumentTile>? = documentOwner.frame?.takeIf{it===expected}?.acquire()
    private fun documentOwnerFrame(snapshot:DocumentSnapshot)=documentOwner.matchingFrame(snapshot,generation)
    private suspend fun documentOwnerDiscardDraw(){documentOwner.discardDraw()}
    private var project:WorkbenchProject?=null;private var changes:Job?=null;private val edits=Mutex();private var drag:Drag?=null
    private var closing=false;private var queued=0;private var colorAssumed=false;private var generation=0L
    private var streamBorrow:WorkbenchProject?=null
    private var streamDocumentId:String?=null
    private var streamEditsSealed=false
    private var streamDisposed=false
    private val streamDisposal=Mutex()
    private fun streamDraftCurrent():Boolean=streamDraftOnly&&!closing&&project!=null&&project===streamBorrow&&
        mutable.value.document?.documentId==streamDocumentId&&mutable.value.isStreamDraft

    /** The native annotation session owns this temporary project and all source
     * pixels. This editor borrows only its document and exact external camera. */
    internal suspend fun installStreamDraft(value:WorkbenchProject,documentId:String,initialCamera:Camera){
        check(streamDraftOnly&&!closing&&streamBorrow==null&&project==null)
        streamBorrow=value;streamDocumentId=documentId;project=value;aiRetiring=true
        require(initialCamera.center.x.isFinite()&&initialCamera.center.y.isFinite()&&
            initialCamera.scale.isFinite()&&initialCamera.scale>0&&initialCamera.rotation.isFinite()&&
            initialCamera.viewportWidth.isFinite()&&initialCamera.viewportWidth>0&&
            initialCamera.viewportHeight.isFinite()&&initialCamera.viewportHeight>0)
        val info=value.info()
        check(documentId in info.documentIds)
        val snapshot=value.document(documentId)
        currentCoroutineContext().ensureActive()
        check(!closing&&project===value&&snapshot.documentId==documentId&&
            snapshot.render.revision.projectId==info.projectId&&snapshot.width>0u&&snapshot.height>0u&&
            !snapshot.hasToneAdjustments&&snapshot.render.items.none{it.shape is Shape.Result})
        generation++;changeSequence=0uL;refreshTicket++
        update{EditorState(document=snapshot,view=ViewState(camera=initialCamera),tool=Tool.Pen,
            projectEpoch=generation,isStreamDraft=true)}
        selections.bind(value,snapshot,generation,changeSequence,initialCamera)
        erasers.bind(value,snapshot,generation,changeSequence,initialCamera)
        changes=scope.launch {
            try{value.changes.collect{event->
                if(streamDraftCurrent()&&project===value&&event.sequence>changeSequence){
                    changeSequence=event.sequence
                    refresh()
                }
            }}catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){if(!closing)report("The annotation revision could not refresh. Keep this draft open or discard it explicitly.")}
        }
    }
    /** Independent gesture owners must finish before Save is admitted. Sealing
     * then retires their idle capabilities and drains accepted ordinary edits
     * without rejecting their existing native transaction. */
    internal suspend fun settleStreamEdits(){
        check(streamDraftCurrent())
        check(ink==null&&drag==null&&markerPress==null&&documentPress==null&&widthGesture==null&&
            !selectionPointer&&!eraserPointer&&mutable.value.textAnchor==null&&
            erasers.state.value.draft==null&&selections.state.value.draft==null&&
            !erasers.state.value.busy&&!selections.state.value.busy&&!selections.saved.value.saving&&
            !rotation.state.value.active&&!rotation.state.value.busy&&
            !instructionEditor.state.value.busy&&!instructionEditor.hasUnsaved()&&
            !semanticEditor.state.value.busy){"Finish the annotation contact and pending editor operations before saving"}
        streamEditsSealed=true
        // Start both retirements before the first suspension can return control
        // to the UI. Their direct public interaction methods then have no live
        // backend, even while an ordinary accepted edit is still draining.
        withContext(NonCancellable){coroutineScope{
            launch(start=CoroutineStart.UNDISPATCHED){erasers.detach()}
            launch(start=CoroutineStart.UNDISPATCHED){selections.detach()}
        }}
        while(queued!=0||inkWork!=null){currentCoroutineContext().ensureActive();delay(4)}
        edits.withLock{check(streamDraftCurrent()&&!mutable.value.busy&&queued==0)}
    }
    /** Failed cleanup keeps the exact borrow installed. Only the native draft
     * owner may close its project after this editor's real retirement. */
    internal suspend fun disposeStreamEditor():Unit=withContext(NonCancellable){streamDisposal.withLock{
        check(streamDraftOnly)
        if(streamDisposed)return@withLock
        streamEditsSealed=true
        retireEditor()
        streamBorrow=null;streamDocumentId=null;streamDisposed=true
    }}
    private val transferFiles=TransferFiles(workRoot);private var transferJob:Job?=null;private var cachedExport:CachedExport?=null
    private var preparedDrag:Pair<CompletedExport,DesktopDragFile>?=null
    private var widthGesture:WidthGesture?=null
    private var ink:DesktopInkRun?=null;private var inkWork:Pair<DesktopInkRun,Job>?=null
    private var refreshTicket=0L;private var changeSequence=0uL
    private val documentRefresh=DocumentRefreshAdmission()
    private var sessionWork:Job?=null;private var sessionProvider:DesktopSessionController?=null
    private var wetCommitted:String?=null
    internal val streamPublication=DesktopStreamPublication(scope,::report)
    private var localTunnelRetirement:suspend()->Unit={}
    internal fun setLocalTunnelRetirement(retire:suspend()->Unit){localTunnelRetirement=retire}
    private val live=DesktopLiveLink(scope,{mutable.value.document?.binding(generation)},::sessionStatus,
        {frame->update{it.copy(peerFrame=frame)}},::report,::detachInstructionFocus,{streamPublication.close();localTunnelRetirement()})
    internal fun remoteCapability():WorkbenchRemoteEdit {
        check(mutable.value.sync?.carrier==SessionCarrier.QuicTether){"Remote editing requires the selected USB tether route"}
        return remoteEdit(checkNotNull(live.attachment()).link)
    }
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
    internal fun aiResultReady(): Boolean {
        val state=mutable.value
        if(state.background!=null&&state.document?.binding(generation)==state.composedBackground)return true
        if(giantDraw.get()?.takeIf{it.content==GiantTileContent.Composed&&giantFrameCurrent(it)}!=null)return true
        return state.document?.render?.items?.none{it.shape is Shape.Result} != false || aiResultFrame()!=null
    }
    fun openAi() {
        if(streamDraftOnly||streamEditsSealed)return
        if(workflows.blocksInput()||frames.state.value.blocksInput)return
        if(documents.state.value.descriptor!=null){report("AI image edits require a raster image document.");return}
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
    private data class MarkerPress(val binding: RenderBinding, val first: Point, val camera: Camera, val invertSnapping: Boolean, val physical:Transform, var end: Point = first)
    private var markerPress: MarkerPress? = null
    val instructionEditor = InstructionEditor(scope, edits,
        { val p = project; val d = mutable.value.document; if (p != null && d != null && !closing && !streamEditsSealed) InstructionAttachment(p, d) else null },
        { core.newId(System.currentTimeMillis().toULong()) },
        { p -> val info = p.info(); val now = System.currentTimeMillis(); WorkflowMetadata(core.newId(now.toULong()), deviceId, info.nextLamport, now) },
        { refresh() }, InstructionEntryMethod.PcKeyboard, localFocused = ::localInstructionFocused)
    val semanticEditor = SemanticEditor(scope, edits,
        { val p = project; val d = mutable.value.document; if (p != null && d != null && !closing && !streamEditsSealed) InstructionAttachment(p, d) else null },
        { val s = mutable.value; !closing && !streamEditsSealed && !s.busy && !ai.state.value.open && queued == 0 && drag == null && ink == null &&
            !instructionEditor.hasUnsaved() && !instructionEditor.state.value.busy && instructionAdmission.allows(s.textAnchor != null, s.exportOpen, s.pasteOpen) },
        { p -> val info = p.info(); val now = System.currentTimeMillis(); WorkflowMetadata(core.newId(now.toULong()), deviceId, info.nextLamport, now) },
        { refresh() },
        { placement -> instructionEditor.place(placement.point, placement.style, placement.bounds, placement.elementEids,
            placement.snapshotId, placement.binding) }, interactionChanged = ::instructionInteractionChanged)
    private val instructionAdmission = com.visualworkbench.desktop.instructions.InstructionFocusAdmission { instructionInteractionChanged() }
    val instructionFocus = InstructionFocusController(scope, instructionEditor,
        { val p = project; val d = mutable.value.document; if (p != null && d != null && !closing && !streamEditsSealed) InstructionAttachment(p, d) else null },
        { !closing && !streamEditsSealed && !mutable.value.busy && !semanticEditor.state.value.busy && drag == null && ink == null && widthGesture == null &&
            !ai.state.value.open && instructionAdmission.allows(mutable.value.textAnchor != null, mutable.value.exportOpen, mutable.value.pasteOpen) },
        { id, peer -> update { it.copy(selected = setOf(id), canvasFocused = if (peer) false else it.canvasFocused) } },
        selectedObject = { mutable.value.selected.singleOrNull() })
    private fun localInstructionFocused(field: InstructionField) { instructionFocus.localField(field) }
    private suspend fun detachInstructionFocus() { instructionFocus.detach() }
    private fun instructionInteractionChanged() { instructionFocus.interactionChanged() }
    fun instructionModal(value: Boolean) { instructionAdmission.modal = value }
    fun instructionPicker(): AutoCloseable = instructionAdmission.picker()
    fun editInstruction() { cancelInput(); mutable.value.selected.firstOrNull()?.let(instructionEditor::focusObject) ?: instructionEditor.focusGlobal() }
    fun mayCloseInstructions(allowFrameRetirementRetry:Boolean=false): Boolean {
        if(!workflows.mayLeave()||!mayLeaveFrames(allowFrameRetirementRetry))return false
        if (!instructionEditor.hasUnsaved()) return true
        report("Save or discard the instruction draft before closing or replacing this project."); return false
    }
    fun sealInstructionsForClose(frameDepartureStarted:Boolean=false): Boolean {
        if(!workflows.mayLeave()||(!frameDepartureStarted&&!mayLeaveFrames()))return false
        if (instructionEditor.sealIfClean()) return true
        report("Save or discard the instruction draft before closing or replacing this project."); return false
    }
    private var selectionPointer=false
    private var eraserPointer=false
    internal val selections=SelectionController(scope,core,workRoot.resolve("selections"),{
        if(!closing&&project!=null)refresh()
    })
    internal val erasers=EraserController(core,scope,selections.interaction){receipt->
        if(!closing&&project!=null){
            if(receipt!=null)update{current->current.copy(selected=current.selected.mapNotNull{id->
                if(id in receipt.replacements)receipt.replacements[id] else id.takeIf{it !in receipt.removed}
            }.toSet())}
            refresh()
        }
    }
    private fun workflowAttachment():EditorWorkflowAttachment? {
        val p=project;val doc=mutable.value.document
        return if(p!=null&&doc!=null&&!closing&&!streamDraftOnly)EditorWorkflowAttachment(p,doc,generation,changeSequence,deviceId)else null
    }
    private fun workflowIdle():Boolean {
        val s=mutable.value
        return !closing&&!streamEditsSealed&&!s.busy&&queued==0&&!s.exportOpen&&!s.pasteOpen&&s.textAnchor==null&&
            !ai.state.value.open&&!ai.needsDecision()&&!instructionEditor.state.value.busy&&!instructionEditor.hasUnsaved()&&
            !semanticEditor.state.value.busy&&instructionAdmission.allows(false,false,false)&&
            drag==null&&ink==null&&inkWork==null&&widthGesture==null&&markerPress==null&&documentPress==null&&
            !selectionPointer&&!eraserPointer&&!rotation.state.value.active&&!rotationUnavailable()&&!selections.state.value.busy&&!erasers.state.value.busy&&
            !documents.state.value.busy&&!frames.state.value.blocksInput&&transferJob?.isActive!=true&&sessionWork?.isActive!=true
    }
    private fun drawingLayer(doc:DocumentSnapshot?):LayerInfo? {
        val layer=doc?.let{documentControlDrawingLayer(it,workflows.controls.state.value.drawingLayerId)}
        if(layer==null)report("Choose a visible unlocked annotation layer before drawing.")
        return layer
    }
    internal fun workflowId():String=core.newId(System.currentTimeMillis().toULong())
    internal val workflows=EditorWorkflows(scope,edits,::workflowAttachment,::workflowIdle,{cancelInput();update{it.copy(view=it.view.copy(followPeer=false))}},
        metadata={p->val info=p.info();val now=System.currentTimeMillis();WorkflowMetadata(core.newId(now.toULong()),deviceId,info.nextLamport,now)},
        newId=::workflowId,camera={mutable.value.view.camera},
        applyCamera={value->update{it.copy(view=it.view.copy(camera=value,followPeer=false))}},
        updated={a,_->if(project===a.project&&generation==a.epoch&&!closing)refresh()},
        committed={a,_->if(project===a.project&&generation==a.epoch&&!closing)refresh()},
        adoptDocument=::adoptWorkflowDocument,report=::report,
        localViews=::localViewWorkflows,documentControls=::documentControlWorkflows,reviews=::offlineReviews,toneControls=::toneControlWorkflows)
    internal val frames=ImageFramesInteraction(scope,edits,::workflowId,
        metadata={p->val info=p.info();val now=System.currentTimeMillis();WorkflowMetadata(core.newId(now.toULong()),deviceId,info.nextLamport,now)},
        mayMutate={val s=mutable.value;!closing&&!streamEditsSealed&&!s.busy&&queued==0&&!workflows.blocksInput()&&
            !ai.state.value.open&&!instructionEditor.state.value.busy&&!semanticEditor.state.value.busy&&
            instructionAdmission.allows(s.textAnchor!=null,s.exportOpen,s.pasteOpen)},
        committed={if(!closing&&project!=null)refresh()},capability=::imageFrameWorkflows)
    private fun mayLeaveFrames(allowRetirementRetry:Boolean=false):Boolean {
        val state=frames.state.value
        if(state.mayLeave||(allowRetirementRetry&&state.departing&&state.retirementPending&&!state.busy))return true
        report("Save, discard or settle the output-frame draft before leaving this project.")
        return false
    }
    internal fun activateFrames(active:Boolean){
        val document=mutable.value.document?:return
        if(active){
            if(documents.state.value.descriptor!=null){report("Output frames require an ordinary image document.");return}
            if(giantDisplayRequired(document)){report("Output frames are unavailable for giant originals. The original remains unchanged.");return}
            if(!workflowIdle().let{idle->idle||frames.state.value.active})return
            cancelInput()
        }
        if(!frames.active(active)&&!active)mayLeaveFrames()
    }
    private var rotationGesture:ObjectRotationGesture?=null
    private var rotationPhysical:Transform?=null
    private val rotationDisplay=java.util.concurrent.atomic.AtomicReference<ObjectRotationHandle?>(null)
    private var rotationPreviewIds:Set<String> = emptySet()
    internal val rotation=ObjectRotationInteraction(scope,edits,::workflowId,::objectRotationTransforms,
        current={val p=project;val s=mutable.value;if(p!=null&&s.document!=null&&!closing)ObjectRotationAttachment(p,s.document,generation,s.view.camera,s.selected)else null},
        mayMutate={val s=mutable.value;!closing&&!streamEditsSealed&&!s.busy&&queued==0&&!workflows.blocksInput()&&!frames.state.value.blocksInput&&!instructionEditor.state.value.busy&&!semanticEditor.state.value.busy&&!ai.state.value.open&&instructionAdmission.allows(s.textAnchor!=null,s.exportOpen,s.pasteOpen)},
        commit={request->
            val doc=mutable.value.document
            if(project!==request.project||doc==null||doc.objectRotationFrame(generation,mutable.value.view.camera)!=request.frame)false
            else {val accepted=performEdit(request.commands,doc.binding(generation),request.gestureId);if(accepted)refresh();accepted}
        })
    internal fun rotationHandle(state:EditorState):ObjectRotationHandle? {
        if(state.tool!=Tool.Select||workflows.blocksInput()||frames.state.value.blocksInput||state.renderIssue!=null)return null
        val base=state.document?.let{objectRotationHandle(it,state.selected,state.view.camera)}?:return null
        if(state.document!==mutable.value.document||state.view.camera!=mutable.value.view.camera)return null
        return rotationDisplay.get()?:base
    }
    private fun rotationUnavailable():Boolean=rotation.state.value.busy||rotation.state.value.failure==ObjectRotationFailure.Unconfirmed||(rotationPreviewIds.isNotEmpty()&&!rotation.state.value.active&&!rotation.state.value.busy)
    private fun startRotation(state:EditorState,point:Point,raw:Point,physical:Transform):Boolean {
        val handle=rotationHandle(state)?:return false
        if(!handle.hit(physical,raw,12.0))return false
        val token=rotation.begin(handle.pivot,point)?:return false
        rotationGesture=token;rotationPhysical=physical;rotationDisplay.set(handle.copy(point=point));return true
    }
    private fun bindWorkflows(reconcileRotation:Boolean=true){if(reconcileRotation)rotation.reconciled();workflowAttachment()?.let{a->workflows.bind(a);frames.bind(a.project,a.document,a.epoch,a.sequence)}}
    private suspend fun enterWorkflows(){workflowAttachment()?.let{workflows.enter(it)}}
    private suspend fun adoptWorkflowDocument(a:EditorWorkflowAttachment,target:DocumentSnapshot){
        if(project!==a.project||generation!=a.epoch||mutable.value.document?.binding(generation)!=a.document.binding(a.epoch)||closing)
            throw WorkflowFailure(WorkflowFailureKind.Stale)
        cancelInput();update{it.copy(view=it.view.copy(followPeer=false))}
        var metadataRetired=false
        withDocumentObserversRetired(
            retire={live.documentChanging()},
            restore={if(!closing&&project===a.project){
                try {
                    if(metadataRetired){instructionEditor.resume();semanticEditor.resumeAfterReplacement();instructionEditor.refresh();instructionFocus.revisionChanged()}
                } finally {live.documentChanged();live.attachment()?.link?.let{instructionFocus.attach(it)}}
            }},
        ) {
            viewportJob?.cancelAndJoin();viewportJob=null;sessionWork?.cancelAndJoin();sessionWork=null
            discardExport();frames.beginDeparture();frames.detach();rotation.detach();semanticEditor.detach();instructionEditor.detach();instructionFocus.detach()
            metadataRetired=true
            erasers.detach();selections.detach();retireAi();detachGiant();documentOwner.detach()
            if(closing||project!==a.project||generation!=a.epoch)throw WorkflowFailure(WorkflowFailureKind.Stale)
            val actual=a.project.info();val expected=target.render.revision
            if(actual.projectId!=expected.projectId||actual.hostSeq!=expected.hostSeq||actual.stateHash!=expected.stateHash||target.documentId !in actual.documentIds){
                refresh();throw WorkflowFailure(WorkflowFailureKind.Stale)
            }
            generation++;refreshTicket++;documentRefresh.invalidate();colorAssumed=false;approvedGiantSourceId=null;aiRetiring=false
            if(!precisionClosed)precision.reset()
            update{EditorState(document=target,path=it.path,view=it.view.copy(peer=null,followPeer=false),projectEpoch=generation,
                canvasFocused=it.canvasFocused,sync=it.sync,sessionRole=it.sessionRole,routeWarnings=it.routeWarnings,shareViewport=it.shareViewport,
                exportSettings=ExportSettings(encoding=if(target.bitDepth>8u)ExportEncoding.Png16 else ExportEncoding.Png8))}
            instructionEditor.resume();semanticEditor.resumeAfterReplacement();metadataRetired=false
            selections.bind(a.project,target,generation,changeSequence,mutable.value.view.camera)
            erasers.bind(a.project,target,generation,changeSequence,mutable.value.view.camera)
            instructionEditor.refresh();instructionFocus.revisionChanged();semanticEditor.refresh();documentOwner.bind(a.project,target)
            live.attachment()?.link?.let{instructionFocus.attach(it)}
            fit();bindWorkflows();refreshOriginalFormatNotice();loadBackground();refreshRole(a.project,generation)
        }
    }
    fun chooseEraser(value:EraserChoice){cancelInput();erasers.tools.choose(value,mutable.value.selected.singleOrNull());update{it.copy(textAnchor=null)}}
    fun stopErasing(){cancelInput();erasers.tools.drawing()}
    private fun update(f:(EditorState)->EditorState){val before=mutable.value;var next=f(before);
        if(rotationGesture!=null&&(next.document!==before.document||next.selected!=before.selected||next.view.camera!=before.view.camera)){rotationGesture=null;rotation.cancel()}
        if(!closing&&!synchronizingPrecision&&next.view.camera.viewportWidth>1&&next.view.camera.viewportHeight>1&&precision.baseCamera()!=next.view.camera){
            synchronizingPrecision=true
            try{if(panningView)cancelPrecision()else cancelInput();precision.synchronize(next.view.camera);next=next.copy(precision=precision.frame(),preview=emptyMap(),previewStyles=emptyMap(),draft=null,wetStroke=mutable.value.wetStroke)}finally{synchronizingPrecision=false}
        }
if(giantMode&&next.document!==before.document)giantOwner.invalidate()
        if(rotation.state.value.preview.isNotEmpty())next=next.copy(preview=next.preview+rotation.state.value.preview)
        next=next.copy(giantActive=giantMode,giantViewport=if(giantMode)runCatching{giantViewport(core,next.view.camera,next.precision)}.getOrNull()else null)
if(preparedDrag?.first?.request?.matches(next)==false){preparedDrag?.second?.close();preparedDrag=null;next=next.copy(dragPrepared=false)};if(next.exportCheck?.request?.matches(next)==false)next=next.copy(exportCheck=null);mutable.value=next;if(next.document!==before.document)ai.changed();if(next.document!==before.document||next.view.camera!=before.view.camera||next.preview!=before.preview||next.peerFrame!=before.peerFrame)aiResults.request();selections.viewport(next.view.camera);erasers.viewport(next.view.camera);documents.viewport(next.view.camera);workflows.remember(next.view.camera);syncGiant();if(next.shareViewport&&(next.view.camera!=before.view.camera||!before.shareViewport))scheduleViewport()}
    /** Select an editor source by identity only. Admission happens under edits. */
    internal fun ownsVerificationOriginal(expected:PackageInfo):Boolean {
        val document=mutable.value.document?:return false
        return !closing&&project!=null&&document.documentId==expected.binding.documentId&&
            document.render.revision.projectId==expected.binding.projectId
    }
    /** Borrow the current original for private history before its pair is known. */
    internal suspend fun <T> withCurrentVerificationOriginal(body:suspend(WorkbenchProject)->T):T = edits.withLock {
        val view=mutable.value;val document=view.document?:throw PackageFailure(PackageFailureKind.Invalid)
        if(closing||view.busy||ai.state.value.open||semanticEditor.state.value.busy||instructionEditor.state.value.busy||drag!=null||ink!=null||inkWork!=null||widthGesture!=null||instructionEditor.hasUnsaved())throw PackageFailure(PackageFailureKind.Busy)
        val current=project?:throw PackageFailure(PackageFailureKind.Closed)
        val expected=document.render.revision;val epoch=generation;val actual=current.info()
        if(closing||project!==current||generation!=epoch||mutable.value.document?.documentId!=document.documentId)throw PackageFailure(PackageFailureKind.Stale)
        if(actual.projectId!=expected.projectId||actual.hostSeq!=expected.hostSeq||actual.stateHash!=expected.stateHash||document.documentId !in actual.documentIds)throw PackageFailure(PackageFailureKind.Stale)
        body(current)
    }
    internal suspend fun <T> withVerificationOriginal(expected:PackageInfo,body:suspend(WorkbenchProject)->T):T = withCurrentVerificationOriginal { current ->
        val document=mutable.value.document?:throw PackageFailure(PackageFailureKind.Stale)
        val actual=current.info();val binding=expected.binding
        if(closing||document.documentId!=binding.documentId||actual.projectId!=binding.projectId||actual.hostSeq!=binding.hostSeq||actual.stateHash!=binding.stateHash)throw PackageFailure(PackageFailureKind.Stale)
        body(current)
    }
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
    internal fun useSessionService(value:DesktopSessionController){check(!streamDraftOnly);sessionProvider=value}
    /** Bind this explicit local choice to the same project/document/link while
     * the picker is open; a reconnect never redirects the selected source. */
    internal fun liveCaptureDestination(): (suspend (com.visualworkbench.bindings.host.CaptureSelection,String,String,String)->Unit)? {
        val target=project?:return null
        val attached=live.attachment()?:return null
        val document=mutable.value.document?.documentId?:return null
        val epoch=generation
        if(mutable.value.sessionRole?.isHost!=true||attached.status.status !in setOf(SyncStatus.Syncing,SyncStatus.Synced))return null
        val connection=try {val focus=markerFocus(attached.link);LiveCaptureConnectionFence(attached){focus.signal().connectionEpoch}}
            catch(_:Exception){return null}
        fun current()=!closing&&project===target&&generation==epoch&&live.attachment()===attached&&
            mutable.value.document?.documentId==document&&connection.current()
        return { selection,name,path,hash->
            if(!current())throw SessionFailure(SessionFailureKind.Closed)
            val local=selection.localSelection()
            streamPublication.start(attached.link,local.publicationSource(core.newId(System.currentTimeMillis().toULong()),name),
                CaptureLocalOptions(true,local,path,hash),::current)
        }
    }
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
    private fun task(epoch: Long? = null, allowFrameDeparture:Boolean=false, block: suspend () -> Unit): Job? {
        if (closing||streamEditsSealed) return null
        if(workflows.blocksInput()||(!allowFrameDeparture&&frames.state.value.blocksInput)||rotationUnavailable()){report("Close the current workflow panel or finish its operation first.");return null}
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
                            is DocumentFailure -> documentFailureText(error)
                            is CoreFailure -> if(error.kind==CoreFailureKind.ImageCodecUnavailable) "A compatible image decoder is unavailable on this system. The original is preserved." else "Core ${error.kind.name.lowercase()}. The last accepted revision is preserved."
                            is SessionFailure -> sessionMessage(error)
                            is UnsupportedOperationException -> error.message ?: "This document feature is unavailable."
                            else -> "The operation could not finish. Check diagnostics and retry."
                        })
                    } finally { update { it.copy(busy = false) } }
                }
            } finally { queued-- }
        }
    }
    private fun transfer(epoch:Long,block:suspend()->Unit):Job?{if(transferJob?.isActive==true){report("A transfer is already running. Finish or cancel it first.");return null};val job=task(epoch,block=block);transferJob=job;return job}
    private suspend fun discardExport(){preparedDrag?.second?.close();preparedDrag=null;val prior=cachedExport;cachedExport=null;update{it.copy(exportReceipt=null,exportCheck=null,dragPrepared=false)};prior?.workspace?.close()}
    private suspend fun attach(value:WorkbenchProject,path:Path){var adopted=false;var instructionSeal=false;try{check(!streamDraftOnly){"The annotation editor cannot replace its borrowed project"};if(!sealInstructionsForClose())return;instructionSeal=true;if(!mayCloseAi())return;val info=value.info();val doc=info.documentIds.firstOrNull()?:error("Project has no document");val snapshot=value.document(doc);cancelInput();live.close();changes?.cancelAndJoin();discardExport();frames.beginDeparture();frames.detach();workflows.detach();rotation.detach();semanticEditor.detach();instructionEditor.detach();erasers.detach();selections.detach();retireAi();detachGiant();documentOwner.detach();if(!precisionClosed)precision.reset();project?.close();project=null;if(closing)return;generation++;changeSequence=0uL;refreshTicket++;project=value;aiRetiring=false;adopted=true;colorAssumed=false;approvedGiantSourceId=null;update{EditorState(path=path,view=it.view.copy(peer=null),document=snapshot,projectEpoch=generation,canvasFocused=it.canvasFocused,busy=true,exportSettings=ExportSettings(encoding=if(snapshot.bitDepth>8u)ExportEncoding.Png16 else ExportEncoding.Png8))};selections.bind(value,snapshot,generation,changeSequence,mutable.value.view.camera);erasers.bind(value,snapshot,generation,changeSequence,mutable.value.view.camera);instructionEditor.refresh();instructionFocus.revisionChanged();semanticEditor.refresh();documentOwner.bind(value,snapshot);fit();bindWorkflows();enterWorkflows();refreshOriginalFormatNotice();loadBackground();refreshRole(value,generation)
        changes=scope.launch{try{value.changes.collect{event->if(project===value&&event.sequence>changeSequence){giantOwner.observe(event.project);workflows.observe(event.project,event.sequence);frames.observe(event.sequence);erasers.observe(event);selections.observe(event);changeSequence=event.sequence;val current=mutable.value.document?.render?.revision;if(current!=null&&(event.project.hostSeq!=current.hostSeq||event.project.stateHash!=current.stateHash)&&mutable.value.composedBackground!=null)update{it.copy(background=null,canvasReady=false)};if(current!=null&&(event.project.hostSeq!=current.hostSeq||event.project.stateHash!=current.stateHash||event.project.nextLamport!=current.nextLamport))refresh()else bindWorkflows(false)}}}catch(cancel:CancellationException){throw cancel}catch(_:Exception){report("The saved revision could not be refreshed. Reopen the project to retry.")}}
        }finally{withContext(NonCancellable){
            try { if(!adopted&&value!==streamBorrow)value.close() }
            finally { if(instructionSeal&&!closing){instructionEditor.detach();instructionEditor.resume();semanticEditor.resumeAfterReplacement();instructionEditor.refresh();instructionFocus.revisionChanged()} }
        }}
    }
    private suspend fun refreshOriginalFormatNotice(){
        if(streamDraftOnly)return
        if(documents.state.value.descriptor!=null){update{it.copy(originalFormatNotice=null,originalFormatInfo=null,originalFormatUnavailable=false)};return}
        val owner=project?:return
        val formats=owner as? WorkbenchOriginalFormats?:return
        val snapshot=mutable.value.document?:return
        val revision=snapshot.render.revision
        val binding=WorkflowBinding(revision.projectId,snapshot.documentId,revision.hostSeq,revision.stateHash)
        val epoch=generation;val ticket=refreshTicket
        fun current():Boolean = !closing&&project===owner&&generation==epoch&&refreshTicket==ticket&&
            mutable.value.document?.let{it.documentId==binding.documentId&&originalFormatRevisionMatches(it.render.revision,binding)}==true
        if(!current())return
        update{it.copy(originalFormatNotice=null,originalFormatInfo=null,originalFormatUnavailable=false)}
        try{
            val receipt=formats.originalFormatInfo(binding)
            currentCoroutineContext().ensureActive()
            validateOriginalFormatInfo(receipt,binding)
            if(current())update{it.copy(originalFormatNotice=originalFormatNotice(receipt),originalFormatInfo=receipt,originalFormatUnavailable=false)}
        }catch(cancel:CancellationException){throw cancel}
        catch(_:Exception){
            currentCoroutineContext().ensureActive()
            if(current())update{it.copy(originalFormatNotice="Original-image page information could not be checked. Reopen the project to retry.",originalFormatInfo=null,originalFormatUnavailable=true)}
        }
    }
    private suspend fun loadBackground(forceGiantReplace:Boolean=false){
        if(streamDraftOnly){check(streamDraftCurrent());return}
        val p=project?:return;val doc=mutable.value.document?:return;val epoch=generation;val assumed=colorAssumed;val ticket=refreshTicket
        if(documents.state.value.descriptor!=null){detachGiant();update{it.copy(background=null,composedBackground=null,composedGuideIds=emptySet(),needsColorConsent=false,backgroundIssue=null,backgroundFailureKind=null)};return}
        if(giantDisplayRequired(doc)){loadGiantBackground(p,doc,forceGiantReplace);return}
        if(giantMode||giantOwner.retaining)detachGiant()
        val renderBinding=doc.binding(epoch)
        val revision=doc.render.revision
        val workflowBinding=WorkflowBinding(revision.projectId,doc.documentId,revision.hostSeq,revision.stateHash)
        fun current()=!closing&&project===p&&generation==epoch&&refreshTicket==ticket&&
            mutable.value.document?.binding(epoch)==renderBinding&&mutable.value.documentPage==(doc.page?:0u)
        if(!current())return
        if(doc.hasToneAdjustments&&!assumed&&mutable.value.originalFormatInfo?.takeIf{it.binding==workflowBinding}?.untagged==true){
            update{it.copy(needsColorConsent=true,backgroundIssue="Tone display requires explicit sRGB consent for this untagged original. The original remains unchanged.",backgroundFailureKind=null)}
            return
        }
        update{it.copy(backgroundIssue=null,backgroundFailureKind=null)}
        try{
            val composed=doc.hasToneAdjustments
            val guideIds=if(composed&&doc.render.items.any{it.shape is Shape.Polygon}){
                composedSelectionGuideIds(doc,documentControlWorkflows(p).snapshot(doc.documentId),workflowBinding)
            }else emptySet()
            val pixels=if(composed){
                val value=imageFrameWorkflows(p).editingBackground(workflowBinding,assumed)
                if(value.binding!=workflowBinding||value.width!=doc.width||value.height!=doc.height||
                    value.sourceBitDepth.toUInt()!=doc.bitDepth||value.sourceAssetId.length!=64||
                    value.rgba.size.toULong()!=doc.width.toULong()*doc.height.toULong()*4uL)
                    throw WorkflowFailure(WorkflowFailureKind.Stale)
                BackgroundImage(value.width,value.height,value.rgba,value.sourceAssetId,value.sourceBitDepth,value.iccProfile)
            }else p.background(doc.documentId,256uL*1024uL*1024uL,assumeUntaggedSrgb=assumed)
            mutable.value.originalFormatInfo?.takeIf{it.binding==workflowBinding}?.let{
                if(it.sourceAssetId!=pixels.sourceAssetId)throw WorkflowFailure(WorkflowFailureKind.Stale)
            }
            currentCoroutineContext().ensureActive()
            val image=withContext(Dispatchers.Default){bitmap(pixels)}
            currentCoroutineContext().ensureActive()
            if(current())update{it.copy(background=image,composedBackground=renderBinding.takeIf{composed},
                composedGuideIds=guideIds,needsColorConsent=false,backgroundIssue=null,backgroundFailureKind=null)}
        }catch(cancel:CancellationException){throw cancel}
        catch(error:CoreFailure){
            currentCoroutineContext().ensureActive()
            if(current())update{it.copy(needsColorConsent=error.kind==CoreFailureKind.Raster&&!assumed,
                backgroundIssue=backgroundFailureMessage(error.kind),backgroundFailureKind=error.kind)}
        }catch(error:Exception){
            currentCoroutineContext().ensureActive()
            if(current())update{it.copy(backgroundIssue=if(error is WorkflowFailure)
                "The saved tone composition could not be displayed. Retry or reopen before editing; saved data is intact."
                else "Image display could not finish. The project remains open; retry the image display.",backgroundFailureKind=null)}
        }
    }
    fun retryBackground():Job?=if(streamDraftOnly)null else task(generation){
        retryWithFreshOriginalFormat(::refreshOriginalFormatNotice){loadBackground(forceGiantReplace=true)}
    }
    fun assumeSrgb(){task(generation){
        val doc=mutable.value.document
        if(doc!=null&&giantDisplayRequired(doc)){
            val revision=doc.render.revision
            val binding=WorkflowBinding(revision.projectId,doc.documentId,revision.hostSeq,revision.stateHash)
            val info=mutable.value.originalFormatInfo?.takeIf{it.binding==binding&&it.untagged}
            if(info==null){report("This untagged source could not be verified. Retry display before approving its colors.");return@task}
            approvedGiantSourceId=info.sourceAssetId
        }else colorAssumed=true
        loadBackground()
    }}
    fun importEditable(path:Path,password:String?,epoch:Long=generation):Job? = transfer(epoch){
        val suffix=path.fileName.toString().substringAfterLast('.',"").lowercase(java.util.Locale.ROOT)
        val kind=when(suffix){"pdf","ai"->EditableKind.Pdf;"svg"->EditableKind.Svg;else->throw DocumentFailure(DocumentFailureKind.Invalid)}
        createEditable(path,path.fileName.toString(),kind,password)
    }
    private suspend fun createEditable(path:Path,title:String,kind:EditableKind,password:String?){
        documents.settle();documentOwner.discardDraw()
        documentOwner.ensureJobs()
        val bytes=withContext(Dispatchers.IO){
            java.nio.channels.Channels.newInputStream(Files.newByteChannel(path,setOf(java.nio.file.StandardOpenOption.READ,java.nio.file.LinkOption.NOFOLLOW_LINKS))).use{input->
                val cap=if(kind==EditableKind.Svg)4*1024*1024 else 64*1024*1024
                val output=ByteArrayOutputStream();val buffer=ByteArray(65536)
                while(true){ensureActive();val count=input.read(buffer);if(count<0)break;if(count==0||count>cap-output.size())throw DocumentFailure(DocumentFailureKind.Limit);output.write(buffer,0,count)}
                output.toByteArray()
            }
        }
        val inspected=DocumentInteraction(documentOwner.jobs,core).inspect(bytes,kind,password)
        val time=System.currentTimeMillis();val id=core.newId(time.toULong());val destination=location.path.resolve("$id.vwb").toAbsolutePath()
        val value=(core as? WorkbenchDocumentsCore?:throw DocumentFailure(DocumentFailureKind.Unavailable)).createDocument(CreateDocument(destination.toString(),id,core.newId(time.toULong()),deviceId,title.take(256),bytes,inspected.first.sourceBlake3,kind,inspected.second,time,password))
        attach(value,destination)
        if(project!==value)throw HandoffFailure("The imported document could not be adopted. The source remains unchanged.")
    }
    fun saveDocument(copy:DocumentExport,destination:Path):Job? = transfer(generation){
        if(!documents.exportPublicationAvailable)throw DocumentFailure(DocumentFailureKind.Unsupported)
        if(documents.checkedExport()!==copy)throw DocumentFailure(DocumentFailureKind.Stale)
        val current=project as? WorkbenchDocumentsProject?:throw DocumentFailure(DocumentFailureKind.Unavailable)
        val id=documents.state.value.descriptor?.documentId?:throw DocumentFailure(DocumentFailureKind.Stale)
        current.publishDocumentCopy(id,destination.toAbsolutePath().toString(),copy)
        report("Document copy saved.")
    }
    fun importImage(path:Path)=importFile(ImportRequest(generation,path.toAbsolutePath().normalize()))
    fun importFile(request: ImportRequest, onStaged: (Boolean) -> Unit = {}, retainAcceptedDrop: Boolean = false): Job? {
        if(streamDraftOnly){onStaged(false);report("The annotation editor cannot replace its borrowed project.");return null}
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
                } else {
                    val title=request.path.fileName.toString()
                    if(ordinaryImportKind(title.substringAfterLast('.',""))==OrdinaryImportKind.PdfCompatibleAi)
                        createEditable(input,title,EditableKind.Pdf,null)
                    else createImported(input,workspace.directory,title)
                }
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
    private suspend fun createImported(input:Path,work:Path,title:String){if(documents.state.value.descriptor!=null){documents.settle();documentOwner.discardDraw()};val streaming=core as? WorkbenchStreamingCore?:throw HandoffFailure("Streaming import is unavailable in this core binding.");val kind=if('.' in title)ordinaryImportKind(title.substringAfterLast('.'))else OrdinaryImportKind.Image;if(kind==OrdinaryImportKind.Unsupported||kind==OrdinaryImportKind.PdfCompatibleAi)throw HandoffFailure("This source needs a supported image or document import route.");val time=System.currentTimeMillis();val id=core.newId(time.toULong());val destination=location.path.toAbsolutePath().resolve("$id.vwb");withContext(Dispatchers.IO){Files.createDirectories(location.path)};val value=try{createOrdinaryImageProject(kind,streaming,CreateFileProject(destination.toString(),id,core.newId(time.toULong()),core.newId(time.toULong()),deviceId,title.take(256),input.toString(),work.toString(),time))}catch(error:CoreFailure){if(error.kind==CoreFailureKind.Raster)throw HandoffFailure("Image import refused. Images over 50 MP, unsupported profiles and sources exceeding the 256 MiB memory budget are not accepted; the original was not resized.");throw error};attach(value,destination);if(project!==value)throw HandoffFailure("The imported image could not be adopted. The source remains unchanged.")}
    fun showPaste(){if(!mutable.value.busy&&!frames.state.value.blocksInput)update{it.copy(pasteOpen=true)}}
    fun dismissPaste(){update{it.copy(pasteOpen=false)}}
    fun pasteImage(assumeSrgb:Boolean,epoch:Long=generation){if(streamDraftOnly){report("The annotation editor cannot replace its borrowed project.");return};update{it.copy(pasteOpen=false)};transfer(epoch){val image=handoff.paste(assumeSrgb);val workspace=transferFiles.create();try{val input=workspace.stage(image.png);createImported(input,workspace.directory,"Pasted image")}finally{workspace.close()}}}
    fun acceptDrop(paths:List<Path>,onStaged:(Boolean)->Unit={}):Job?{if(paths.size!=1){report("Drop one image, PDF-compatible AI or MP4 file at a time.");onStaged(false);return null};val path=paths.single();val extension=path.fileName.toString().substringAfterLast('.',"").lowercase();if(extension!="mp4"&&ordinaryImportKind(extension)==OrdinaryImportKind.Unsupported){report("This drop supports admitted images, embedded RAW previews, flattened PSD/PSB composites, PDF-compatible AI and ordinary MP4 assets.");onStaged(false);return null};return importFile(ImportRequest(generation,path.toAbsolutePath().normalize()),onStaged,retainAcceptedDrop=true)}
    fun open(path:Path,epoch:Long=generation){if(streamDraftOnly){report("The annotation editor cannot replace its borrowed project.");return};task(epoch){attach(core.open(path.toAbsolutePath().toString()),path)}}
    suspend fun close(){if(streamDraftOnly)disposeStreamEditor()else retireEditor()}
    private suspend fun retireEditor(){closing=true;frames.beginDeparture();workflows.beginDeparture();giantOwner.invalidate();transferJob?.cancel();sessionWork?.cancel();viewportJob?.cancel();cancelInput();withContext(NonCancellable){edits.withLock{live.close();changes?.cancelAndJoin();changes=null;discardExport();frames.close();workflows.close();rotation.close();semanticEditor.detach();instructionEditor.detach();erasers.detach();selections.detach();retireAi();detachGiant();documentOwner.close();closePrecision();if(!streamDraftOnly)project?.close();project=null;generation++;refreshTicket++;update{EditorState(view=it.view.copy(peer=null),projectEpoch=generation,canvasFocused=it.canvasFocused)}}}}
    fun closeProject(){if(streamDraftOnly){report("Close this annotation through its stream owner.");return};if(!mayCloseAi())return;if(!mayCloseInstructions(allowFrameRetirementRetry=true))return;frames.active(false);cancelInput();sessionWork?.cancel();val epoch=generation;val closingTask=task(epoch,allowFrameDeparture=true){
        // The pre-enqueue check can become stale while another owner holds
        // edits. task has now set busy under that same mutex, fencing Send.
        if(!mayCloseAi()){frames.detach();bindWorkflows(false);return@task}
        if(!sealInstructionsForClose(frameDepartureStarted=true)){frames.detach();bindWorkflows(false);return@task}
        try { live.close();changes?.cancelAndJoin();changes=null;discardExport();frames.detach();workflows.detach();rotation.detach();semanticEditor.detach();instructionEditor.detach();erasers.detach();selections.detach();retireAi();detachGiant();documentOwner.detach();project?.close();project=null;generation++;refreshTicket++;update{EditorState(view=it.view.copy(peer=null),projectEpoch=generation,canvasFocused=it.canvasFocused)} }
        finally { if(!closing){instructionEditor.detach();instructionEditor.resume();semanticEditor.resumeAfterReplacement();instructionEditor.refresh();instructionFocus.revisionChanged()} }
    };if(closingTask!=null){if(project!=null&&generation==epoch)frames.beginDeparture();closingTask.invokeOnCompletion{
        if(!closing&&project!=null&&generation==epoch&&frames.state.value.departing&&
            !frames.state.value.retirementPending)scope.launch{edits.withLock{
                if(!closing&&project!=null&&generation==epoch&&frames.state.value.departing){
                    try{frames.detach();bindWorkflows(false)}catch(_:Exception){report("Output-frame retirement is incomplete. Retry closing the project.")}
                }
            }}
    }}}
    private suspend fun refresh(){
        val p=project?:return;val epoch=generation;val requested=mutable.value.document?:return
        val ticket=++refreshTicket;val observed=changeSequence
        val fetched=documentRefresh.load(requested,{mutable.value.document}){p.document(requested.documentId)}?:return
        val snapshot=fetched.snapshot
        if(project!==p||epoch!=generation||ticket!=refreshTicket||observed<changeSequence||!documentRefresh.accepts(fetched.ticket,mutable.value.document,snapshot))return
        // A host sequence is the accepted journal position, not the version of
        // the client's optimistic view. Confirm that this fetch still matches
        // the worker's current visible hash before admitting a same-seq change.
        val currentInfo=p.info()
        if(project!==p||epoch!=generation||ticket!=refreshTicket||observed<changeSequence||!documentRefresh.accepts(fetched.ticket,mutable.value.document,snapshot))return
        if(currentInfo.hostSeq!=snapshot.render.revision.hostSeq||currentInfo.stateHash!=snapshot.render.revision.stateHash)return
        if(streamDraftOnly)check(snapshot.documentId==streamDocumentId&&!snapshot.hasToneAdjustments&&snapshot.render.items.none{it.shape is Shape.Result})
        when(snapshotDecision(mutable.value.document,snapshot,allowVisibleChange=true)){
            SnapshotDecision.Stale->return
            SnapshotDecision.Duplicate->{if(!streamEditsSealed){selections.bind(p,snapshot,generation,changeSequence,mutable.value.view.camera);erasers.bind(p,snapshot,generation,changeSequence,mutable.value.view.camera)};if(!streamDraftOnly)bindWorkflows();return}
            SnapshotDecision.Conflict->{cancelDrag();update{it.copy(canvasReady=false,renderIssue="The same revision returned different content. Reopen the project before editing.")};return}
            SnapshotDecision.Newer->{
                cancelDrag();cancelWidth();update{it.copy(document=snapshot,selected=it.selected.intersect(snapshot.render.items.filterNot{item->item.locked}.map{item->item.objectId}.toSet()),preview=emptyMap(),previewStyles=emptyMap(),draft=null,peerFrame=null,exportReceipt=null,exportCheck=null,canvasReady=false,renderIssue=null)}
            }
        }
        if(!streamEditsSealed){selections.bind(p,snapshot,generation,changeSequence,mutable.value.view.camera);erasers.bind(p,snapshot,generation,changeSequence,mutable.value.view.camera)}
        if(streamDraftOnly){check(streamDraftCurrent()&&!snapshot.hasToneAdjustments&&snapshot.render.items.none{it.shape is Shape.Result});return}
        instructionEditor.refresh();instructionFocus.revisionChanged();semanticEditor.refresh()
        documentOwner.bind(p,snapshot)
        bindWorkflows()
        refreshOriginalFormatNotice()
        if(giantMode||giantDisplayRequired(snapshot)||snapshot.hasToneAdjustments||mutable.value.composedBackground!=null)loadBackground()
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
    private fun pointerAllowed(panButton:Boolean):Boolean {
        val s=mutable.value
        return !closing&&!streamEditsSealed&&!rotationUnavailable()&&!workflows.blocksInput()&&!frames.state.value.blocksInput&&!ai.state.value.open&&(panButton||s.tool==Tool.Pan||aiResultReady())&&!s.busy&&!instructionEditor.state.value.busy&&!semanticEditor.state.value.busy&&!documents.state.value.busy&&s.document!=null&&(s.background!=null||s.documentFrame!=null||giantInputReady()||streamDraftCurrent())&&s.backgroundIssue==null&&
            (!s.document.hasToneAdjustments||composedInputReady(s))&&
            (s.canvasReady||panButton||s.tool==Tool.Pan)&&s.renderIssue==null&&s.textAnchor==null&&!s.exportOpen&&!s.pasteOpen&&instructionAdmission.allows(false,false,false)
    }
    fun precisionDown(raw:Point,extend:Boolean,panButton:Boolean,uptimeMs:Long,pressure:Float=1f,semanticModifier:Boolean=false){
        if(!pointerAllowed(panButton))return
        val keepPolygon = !panButton && !selectionPointer && !eraserPointer && drag == null && ink == null && markerPress == null && documentPress == null &&
            erasers.tools.route() == EraserInputRoute.Selection && selections.interaction.canResumePolygonContact(mutable.value.view.camera)
        clearInputContact(keepPolygon)
        val state=mutable.value;val pan=panButton||(state.tool==Tool.Pan&&!selections.state.value.active&&!erasers.state.value.active)
        if(pan){pointerDown(raw,extend,panButton,uptimeMs,pressure,semanticModifier);return}
        val physical=precision.inputMatrix(raw);val mapped=precision.route(listOf(raw)).single()
        val documentMode=documents.state.value
        if(documentMode.textTool||documentMode.regionTool){
            val point=core.mapPoints(state.view.camera,true,listOf(mapped)).single()
            documentPress=point to point;documentPressRegion=documentMode.regionTool
            documentPressBinding=documentMode.descriptor?.let{DocumentFrameBinding(it.sourceAssetId,it.documentId,documentMode.page,it.revision,generation,0)}
        }else pointerDown(mapped,extend,false,uptimeMs,pressure,semanticModifier,physical,raw)
        if(rotationGesture!=null||documentPress!=null||eraserPointer||selectionPointer||ink!=null||drag!=null||markerPress!=null||mutable.value.textAnchor!=null){precision.down(raw);precisionContact=true}
    }
    fun precisionMove(raw:Point,uptimeMs:Long,pressure:Float=1f,snapRotation15:Boolean=false){
        if(precisionContact)precision.move(raw)
        val mapped=if(precisionContact)precision.route(listOf(raw)).single()else raw
        rotationGesture?.let{token->if(precision.inputMatrix()!=rotationPhysical){rotationGesture=null;rotation.cancel();return};val point=core.mapPoints(token.frame.camera,true,listOf(mapped)).single();rotationDisplay.get()?.let{rotationDisplay.set(it.copy(point=point))};rotation.move(token,point,snapRotation15);return}
        val press=documentPress
        if(press!=null){documentPress=press.first to core.mapPoints(mutable.value.view.camera,true,listOf(mapped)).single();return}
        pointerMove(mapped,uptimeMs,pressure)
    }
    fun precisionUp(raw:Point,uptimeMs:Long,pressure:Float=1f,snapRotation15:Boolean=false){
        try{
            val mapped=if(precisionContact)precision.route(listOf(raw)).single()else raw
            rotationGesture?.let{token->rotationGesture=null;if(precision.inputMatrix()!=rotationPhysical){rotation.cancel();return};val point=core.mapPoints(token.frame.camera,true,listOf(mapped)).single();rotationDisplay.get()?.let{rotationDisplay.set(it.copy(point=point))};rotation.finish(token,point,snapRotation15);return}
            val press=documentPress
            if(press!=null){
                documentPress=null;val binding=documentPressBinding;documentPressBinding=null
                val current=mutable.value.document
                if(binding!=null&&generation==binding.attachmentEpoch&&current?.render?.revision?.sameDocumentRevision(binding.revision)==true&&documents.state.value.page==binding.page){
                    val end=core.mapPoints(mutable.value.view.camera,true,listOf(mapped)).single()
                    val region=Rect(minOf(press.first.x,end.x),minOf(press.first.y,end.y),kotlin.math.abs(end.x-press.first.x),kotlin.math.abs(end.y-press.first.y))
                    if(documentPressRegion)documents.region(region)else documents.select(region)
                }
            }else pointerUp(mapped,uptimeMs,pressure)
        }finally{precisionContact=false;precision.end()}
    }
    fun precisionHover(raw:Point?){if(!closing)precision.hover(raw)}
    fun pointerDown(viewPoint:Point,extend:Boolean,panButton:Boolean,uptimeMs:Long=System.nanoTime()/1_000_000,pressure:Float=1f,semanticModifier:Boolean=false,physical:Transform?=null,raw:Point=viewPoint){if(streamEditsSealed||closing||rotationUnavailable()||workflows.blocksInput()||frames.state.value.blocksInput||ai.state.value.open||(!panButton&&mutable.value.tool!=Tool.Pan&&!aiResultReady()))return;val s=mutable.value;if(s.busy||instructionEditor.state.value.busy||semanticEditor.state.value.busy||s.document==null||(s.background==null&&s.documentFrame==null&&!giantInputReady()&&!streamDraftCurrent())||s.backgroundIssue!=null||(s.document.hasToneAdjustments&&!composedInputReady(s))||documents.state.value.busy||(!s.canvasReady&&!panButton&&s.tool!=Tool.Pan)||s.renderIssue!=null)return;val binding=s.document.binding(generation);val point=core.mapPoints(s.view.camera,true,listOf(viewPoint)).single();val pan=panButton||(s.tool==Tool.Pan&&!selections.state.value.active&&!erasers.state.value.active)
        if(!pan&&s.tool==Tool.Select&&erasers.tools.route()==EraserInputRoute.Editor&&startRotation(s,point,raw,physical?:core.cameraMatrix(s.view.camera)))return
        if(!pan)when(erasers.tools.route()){
            EraserInputRoute.Selection->{selectionPointer=selections.interaction.begin(viewPoint,s.view.camera);return}
            EraserInputRoute.Eraser->{eraserPointer=erasers.interaction.begin(viewPoint,s.view.camera,physicalScale=physical?.let{kotlin.math.hypot(it.a,it.b)}?:s.view.camera.scale);return}
            EraserInputRoute.Blocked->return
            EraserInputRoute.Editor->Unit
        }
        if(selections.state.value.busy||erasers.state.value.busy)return
        if(s.tool==Tool.Callout&&!pan){markerPress=MarkerPress(binding,point,s.view.camera,semanticModifier,physical?:core.cameraMatrix(s.view.camera));update{it.copy(draft=point to point)};return}
        if(s.tool==Tool.Pen&&!pan){beginInk(DesktopSample(point,uptimeMs,pressure),binding);return}
        if(s.tool==Tool.Text&&!pan){update{it.copy(textAnchor=point)};return}
        if(s.tool !in listOf(Tool.Select,Tool.Pan)&&!pan){update{it.copy(draft=point to point)};drag=Drag(point,s.view,emptyList(),null,false,false,binding,newObject=newObjectPreview(s,binding),tool=s.tool);return}
        var selected=s.document?.render?.items?.filter{it.objectId in s.selected}?:emptyList();val bounds=union(selected.map{it.bounds})
        val resize=!pan&&bounds!=null&&precisionResizeHit(bounds,physical?:core.cameraMatrix(s.view.camera),raw,10.0)
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
        if(d.pan){panningView=true;try{update{it.copy(view=it.view.copy(camera=d.initial.camera.copy(center=Point(d.initial.camera.center.x-dx,d.initial.camera.center.y-dy))))}}finally{panningView=false};return}
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
                val matrix = press.physical
                semanticEditor.place(point, ObjectStyle(current.color, current.width), matrix, semanticDropBox(point, end, matrix), press.invertSnapping)
            }
            else report("The document changed. Place the marker again on the current revision.")
            return
        }
        if(eraserPointer){eraserPointer=false;erasers.interaction.finish(viewPoint);return};if(selectionPointer){selectionPointer=false;selections.interaction.finish(viewPoint);return};if(ink!=null){if(viewPoint!=null)pointerMove(viewPoint,uptimeMs,pressure);finishInk();return};val s=mutable.value;val d=drag;drag=null;if(d==null)return;if(!geometryMatches(d.binding,s)){val old=d.live?:d.newObject;live.cancelLater(old?.attachment,old?.id);cancelDrag();return}
        if(s.draft!=null){val(a,b)=s.draft;val r=Rect(minOf(a.x,b.x),minOf(a.y,b.y),kotlin.math.abs(a.x-b.x),kotlin.math.abs(a.y-b.y));val shape=when(d.tool){Tool.Rectangle->Shape.Rectangle(r);Tool.Ellipse->Shape.Ellipse(r);Tool.Line->Shape.Line(listOf(a,b));Tool.Arrow->Shape.Arrow(listOf(a,b));else->null};update{it.copy(draft=null)};val provisional=d.newObject;val geometry=draftShape(d.tool,a,b);if(provisional!=null){if(geometry!=null)gestureEdit(listOf(EditCommand.Create(provisional.objectId,provisional.layer,geometry,provisional.style)),d.binding,provisional)else live.cancelLater(provisional.attachment,provisional.id)}else if(shape!=null&&r.width+r.height>0.01)create(shape,d.binding);return}
        val edits=s.preview.filter{(id,transform)->d.selected.any{it.objectId==id&&it.transform!=transform}}.map{EditCommand.SetTransform(it.key,it.value)};update{it.copy(preview=emptyMap())};gestureEdit(edits,d.binding,d.live)
    }
    fun cancelDrag(){clearDragContact(false)}
    private fun clearDragContact(retainPolygon:Boolean){precisionContact=false;documentPress=null;documentPressBinding=null;cancelPrecision();markerPress=null;eraserPointer=false;erasers.interaction.cancelGesture();selectionPointer=false;if(!retainPolygon)selections.interaction.cancelGesture();val old=drag?.let{it.live?:it.newObject};drag=null;live.cancelLater(old?.attachment,old?.id);update{it.copy(preview=emptyMap(),draft=null)}}
    private fun newObjectPreview(state:EditorState,binding:RenderBinding):NewObjectPreviewLease?{
        val attachment=live.attachment()?:return null;val shape=draftTemplate(state.tool)?:return null
        val layer=drawingLayer(state.document)?:return null;val now=System.currentTimeMillis()
        return NewObjectPreviewLease(core.newId(now.toULong()),attachment,core.newId(now.toULong()),binding.documentId,layer.id,now,shape,ObjectStyle(state.color,state.width))
    }
    private fun previewFor(items:List<RenderItem>,binding:RenderBinding,kind:PreviewKind):ObjectPreviewLease?{
        val item=items.singleOrNull()?:return null;val attached=live.attachment()?:return null
        return ObjectPreviewLease(core.newId(System.currentTimeMillis().toULong()),attached,binding.documentId,item.objectId,kind)
    }
    private fun cancelWidth(){val old=widthGesture;widthGesture=null;live.cancelLater(old?.live?.attachment,old?.live?.id);update{it.copy(previewStyles=emptyMap())}}
    fun cancelInput(){rotationGesture=null;rotation.cancel();clearInputContact(false)}
    private fun clearInputContact(retainPolygon:Boolean){if(rotationGesture!=null){rotationGesture=null;rotation.cancel()};precisionContact=false;documentPress=null;documentPressBinding=null;cancelPrecision();clearDragContact(retainPolygon);cancelWidth();cancelInk()}
    fun finishPointerContact(){clearDragContact(selections.interaction.canResumePolygonContact(mutable.value.view.camera))}
    private fun beginInk(first:DesktopSample,binding:RenderBinding){
        val s=mutable.value;val p=project?:return;val doc=s.document?:return;val layer=drawingLayer(doc)?:return
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
    internal fun create(shape:Shape,binding:RenderBinding?=mutable.value.document?.binding(generation),afterSuccess:()->Unit={}){val s=mutable.value;if(s.renderIssue!=null)return;val layer=drawingLayer(s.document)?:return;edit(listOf(EditCommand.Create(core.newId(System.currentTimeMillis().toULong()),layer.id,shape,ObjectStyle(s.color,s.width))),binding,afterSuccess)}
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
    fun follow(){if(workflows.blocksInput())return;if(mutable.value.view.peer==null){report("Peer view is unavailable until an authenticated session is connected.");return};update{it.copy(view=it.view.copy(followPeer=!it.view.followPeer))}}
    fun matchPeer(){if(workflows.blocksInput())return;if(mutable.value.view.peer==null)report("No authenticated peer viewport is available.")else update{it.copy(view=it.view.matchPeer())}}
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
    fun showExport(){if(mutable.value.document==null){report("Open a document before exporting.");return};if(frames.state.value.blocksInput){report("Close or settle the output-frame panel before exporting.");return};if(mutable.value.originalFormatUnavailable){report("The original format could not be verified. Reopen the project before exporting.");return};update{it.copy(exportOpen=true)}}
    fun dismissExport(){update{it.copy(exportOpen=false)}}
    fun exportSettings(settings:ExportSettings){if(!settings.encoding.exportAvailable){report("BMP export is unavailable until its file metadata can carry the revision and source identity.");return};update{it.copy(exportSettings=settings.copy(quality=settings.quality.coerceIn(1,100)),message=null)}}
    fun exportRequest():ExportRequest?=try{captureExport(mutable.value,core)}catch(error:HandoffFailure){report(error.message?:"Export region unavailable.");null}
    private fun requireCurrent(request:ExportRequest){if(!request.matches(mutable.value)||mutable.value.originalFormatUnavailable)throw HandoffFailure("The original, project, revision, region or export settings changed. Review them and prepare the export again.")}
    private suspend fun projectedExport(request:ExportRequest):ExportRequest {
        requireCurrent(request)
        val p=project?:throw HandoffFailure("Open a project before exporting.")
        if(!request.matches(p.info())){refresh();throw HandoffFailure("The core revision changed. Review it before exporting.")}
        val projected=try{projectExport(request,p)}catch(error:WorkflowFailure){
            throw HandoffFailure(when(error.kind){
                WorkflowFailureKind.Stale,WorkflowFailureKind.Missing,WorkflowFailureKind.Detached,WorkflowFailureKind.Closed ->
                    "The saved output frame or original changed. Reopen the project and prepare this export again."
                WorkflowFailureKind.Invalid -> "The saved output frame or selected source rectangle is invalid. Review the frame and export area."
                WorkflowFailureKind.Limit,WorkflowFailureKind.Memory,WorkflowFailureKind.Backpressure ->
                    "Output-frame projection exceeds its work or memory allowance. Choose a smaller region or retry when work settles."
                else -> "The saved output frame could not be verified for export. The original is unchanged."
            })
        }
        requireCurrent(request)
        requireCurrent(projected)
        if(!projected.matches(p.info()))throw HandoffFailure("The project changed while resolving its saved output frame.")
        return projected
    }
    private fun transferApi():WorkbenchFileTransfers=project as? WorkbenchFileTransfers?:throw HandoffFailure("File export is unavailable in this core binding.")
    private fun fileOptions(request:ExportRequest,workspace:TransferWorkspace)=FileExportOptions(request.options(),workspace.output.toString(),workspace.directory.toString())
    fun checkExport(request:ExportRequest){transfer(request.epoch){val projected=projectedExport(request);val workspace=transferFiles.create();try{val result=transferApi().preflightFile(fileOptions(projected,workspace));if(!projected.matches(result.revision))throw HandoffFailure("The project changed during preflight. Check it again.");requireCurrent(request);requireCurrent(projected);update{it.copy(exportCheck=ExportCheck(request,result),message="Format preflight passed for ${projected.width} × ${projected.height} output pixels. Marked geometry and result assets are checked again during export.")}}finally{workspace.close()}}}
    private suspend fun prepareExport(request:ExportRequest):CachedExport{
        val projected=projectedExport(request)
        cachedExport?.takeIf{it.receipt.request==projected}?.let{update{state->state.copy(exportReceipt=it.receipt)};return it}
        discardExport();val workspace=transferFiles.create();var retained=false
        try{val api=transferApi();val options=fileOptions(projected,workspace);val preflight=api.preflightFile(options);if(!projected.matches(preflight.revision))throw HandoffFailure("The project changed during preflight.");requireCurrent(request);requireCurrent(projected);update{it.copy(exportCheck=ExportCheck(request,preflight))};val result=api.exportTransfer(options);val completed=completeExport(projected,result);requireCurrent(request);requireCurrent(projected);if(withContext(Dispatchers.IO){Files.size(workspace.output).toULong()}!=completed.bytes)throw HandoffFailure("The completed export size differs from its receipt.");val cached=CachedExport(workspace,completed);cachedExport=cached;retained=true;update{it.copy(exportReceipt=completed)};return cached}finally{if(!retained)workspace.close()}
    }
    fun prepareExportFile(request:ExportRequest){transfer(request.epoch){val cached=prepareExport(request);report("${cached.receipt.revision} prepared at ${cached.receipt.width} × ${cached.receipt.height} px.")}}
    fun saveExport(request:ExportRequest,path:Path){val destination=path.toAbsolutePath().normalize();transfer(request.epoch){val cached=prepareExport(request);publishExport(cached.workspace.output,destination,cached.receipt.bytes){request.matches(mutable.value)};report("${cached.receipt.revision} saved at ${cached.receipt.width} × ${cached.receipt.height} px.")}}
    fun copyExport(request:ExportRequest){transfer(request.epoch){if(!request.settings.encoding.png)throw HandoffFailure("Clipboard copy requires a PNG export. Choose PNG 8 or 16 bit.");val cached=prepareExport(request);requireCurrent(request);handoff.copy(cached.workspace.output,cached.receipt);report("${cached.receipt.revision} copied as PNG and DIBV5.")}}
    fun prepareDragExport(request:ExportRequest){transfer(request.epoch){if(!request.settings.encoding.png)throw HandoffFailure("File drag requires PNG. Choose PNG 8 or 16 bit.");val cached=prepareExport(request);requireCurrent(request);val lease=handoff.stage(cached.workspace.output,cached.receipt);var adopted=false;try{requireCurrent(request);preparedDrag?.second?.close();preparedDrag=cached.receipt to lease;adopted=true;update{it.copy(dragPrepared=true,exportOpen=false,message="PNG drag prepared. Drag the button to the destination; nothing is sent automatically.")}}finally{if(!adopted)lease.close()}}}
    internal fun takePreparedDrag():DesktopDragFile?{val offer=preparedDrag?:return null;if(!offer.first.request.matches(mutable.value)){offer.second.close();preparedDrag=null;update{it.copy(dragPrepared=false)};return null};preparedDrag=null;update{it.copy(dragPrepared=false)};return offer.second}
    fun cancelTransfer(){transferJob?.cancel();report("Cancelling transfer; waiting for native work to release its files.")}
    init {
        scope.launch { rotation.state.collect{value->
            if(!value.active&&!value.busy){rotationDisplay.set(null);rotationPhysical=null}
            val previous=rotationPreviewIds;rotationPreviewIds=value.preview.keys.toSet()
            update{it.copy(preview=(it.preview-previous)+value.preview)}
            if(value.failure==ObjectRotationFailure.Unconfirmed&&!closing)scope.launch{edits.withLock{
                try{refresh()}catch(_:Exception){report("Refresh or reopen the project before rotating again. Any accepted edit remains in history.")}
            }}
        }}
    }
}
private fun <T> Set<T>.toggle(value:T):Set<T> = if(value in this)this-value else this+value
private fun backgroundFailureMessage(kind:CoreFailureKind):String=when(kind){
    CoreFailureKind.Invalid->"The image display request was refused as invalid. The project remains open."
    CoreFailureKind.Backpressure->"Image display was refused by the work queue or fixed 256 MiB budget. Retry when current work finishes."
    CoreFailureKind.Closed->"Image display could not finish because the project was closed. Reopen the saved project to retry."
    CoreFailureKind.Cancelled->"Image display was cancelled. Retry when current work finishes."
    CoreFailureKind.Storage->"Image display could not read project storage. Check the saved project and retry."
    CoreFailureKind.Unsupported->"This original uses an image feature that cannot be displayed here."
    CoreFailureKind.Worker->"Image display could not finish in its worker. Retry the image display."
    CoreFailureKind.Stroke->"Image display could not finish because of a drawing failure. Retry the image display."
    CoreFailureKind.ImageCodecUnavailable->"An installed AVIF decoder is unavailable. Color conversion cannot enable a missing decoder."
    CoreFailureKind.Raster->"The original could not be displayed because of a source, color, decoding or image-processing failure. If the image is untagged, choose sRGB explicitly."
}
private fun bitmap(value:BackgroundImage):ImageBitmap {
    val width=value.width.toInt();val height=value.height.toInt();val image=BufferedImage(width,height,BufferedImage.TYPE_INT_ARGB);val row=IntArray(width)
    for(y in 0 until height){for(x in 0 until width){val i=(y*width+x)*4;row[x]=((value.rgba[i+3].toInt() and 255) shl 24) or ((value.rgba[i].toInt() and 255) shl 16) or ((value.rgba[i+1].toInt() and 255) shl 8) or (value.rgba[i+2].toInt() and 255)};image.setRGB(0,y,width,1,row,0,width)}
    val bytes=ByteArrayOutputStream().use{check(ImageIO.write(image,"png",it));it.toByteArray()};return Image.makeFromEncoded(bytes).toComposeImageBitmap()


}
