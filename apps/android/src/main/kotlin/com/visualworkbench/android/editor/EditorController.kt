@file:OptIn(ExperimentalUnsignedTypes::class)

package com.visualworkbench.android.editor

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Path
import android.net.Uri
import android.view.KeyEvent
import android.view.MotionEvent
import android.os.SystemClock
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import com.visualworkbench.android.diagnostics.DiagnosticsController
import com.visualworkbench.android.input.*
import com.visualworkbench.android.precision.*
import com.visualworkbench.android.airactions.*
import com.visualworkbench.android.assistedselection.AssistedSourceGate
import com.visualworkbench.android.assistedselection.appendAssistedPenPrompt
import com.visualworkbench.android.ui.ProjectCard
import com.visualworkbench.shared.assistedselection.backend.LiteRtReviewedCatalog
import com.visualworkbench.shared.assistedselection.backend.liteRtAssistedSelectionBackend
import com.visualworkbench.shared.*
import com.visualworkbench.android.session.PairingController
import com.visualworkbench.android.session.RefreshAdmission
import com.visualworkbench.android.session.sessionMessage
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.io.File
import kotlin.math.*

internal enum class WorkbenchScreen { Projects, Canvas, Settings, Diagnostics, Pairing, Instructions }
internal data class TextDraft(val objectId: String?, val anchor: Point, val text: String = "", val font: String = "Inter", val size: Double = 32.0)
internal enum class ImportOutcome { Imported, Cancelled, Retained }
internal data class CameraCapture(val token: String, val bytes: Long)

/** A successful open belongs to one native handle and editor generation. */
internal class ProjectOpenCompletion<P : Any>(private val opened: P, private val epoch: Long) {
    fun deliverIfCurrent(current: P?, currentEpoch: Long, onOpened: (P, Long) -> Unit) {
        if (current === opened && currentEpoch == epoch) onOpened(opened, epoch)
    }
}

internal fun projectReceiveFailureMessage(kind: SessionFailureKind): String = when (kind) {
    SessionFailureKind.Authentication -> "The project could not be verified as belonging to the selected computer. Check its pairing and project ownership on the PC."
    SessionFailureKind.Timeout, SessionFailureKind.Transport -> "The computer's project could not be reached. On the PC, open the project and start its connection in Pair devices. For USB, select the connection and this phone, choose Use selected connection for USB project, then Connect phone to this project."
    SessionFailureKind.Storage -> "The received project could not be saved or verified on this phone. Check available storage. Existing saved projects are unchanged."
    SessionFailureKind.Backpressure -> "The project transfer exceeded a current resource or transfer limit. Existing saved projects are unchanged."
    SessionFailureKind.Cancelled -> "Project receive canceled. Existing saved projects are unchanged."
    SessionFailureKind.Closed -> "The project connection closed before receive finished. Reconnect to the selected computer and try again. Existing saved projects are unchanged."
    else -> "The computer project could not be received safely (${kind.name}). Existing saved projects are unchanged."
}

/** UI state lives on Main; native/storage work is suspendable and input queues are bounded. */
internal class EditorController @JvmOverloads constructor(application: Application, val core: WorkbenchCore = workbenchCore(),
    captureStatusFactory: (ProjectLink) -> WorkbenchAgentCaptureStatus = ::agentCaptureStatus,
    private val captureArrivalQuery: suspend (WorkbenchProject, ULong) -> ReceivedCapture? = { p, after -> p.receivedCapture(after) },
    sessionFactory: suspend (android.content.Context, String) -> WorkbenchSessions = ::createAndroidSessions,
    private val assistedCatalog: LiteRtReviewedCatalog = LiteRtReviewedCatalog.empty(File(application.noBackupFilesDir,"assisted-selection")),
    private val assistedPins: AssistedSelectionPins? = null) : AndroidViewModel(application), CanvasGestureSink {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val assistedSource=AssistedSourceGate<WorkbenchProject>()
    internal var assistedActions by mutableStateOf<AssistedSelectionInteraction?>(null); private set
    internal var assistedPanelOpen by mutableStateOf(false); private set
    internal var assistedPrompt by mutableStateOf(AssistedSelectionPrompt()); private set
    internal var assistedSubmittedPrompt by mutableStateOf<AssistedSelectionPrompt?>(null); private set
    internal var assistedNegative by mutableStateOf(false); private set
    private var assistedCollecting=false
    private data class AssistedTap(val project:WorkbenchProject,val source:AssistedSelectionSource,
        val camera:Camera,val down:PenSample,val negative:Boolean)
    private var assistedTap:AssistedTap?=null
    private var assistedNewObjectId:String?=null
    private var assistedOpening=false
    private val airEpoch = AtomicLong()
    private var airInitialized = false
    private val airViewRestore = AirActionViewRestore()
    private val airBarrel = AirActionBarrelCorrelation()
    private var applyingAirView = false
    private var airActivity: Any? = null
    private var airResumed by mutableStateOf(false)
    private var airActivityFocused by mutableStateOf(false)
    private var airRemoteFocused by mutableStateOf(false)
    private var airKeyReceiver: ((KeyEvent) -> Boolean)? = null
    private var airSchedule: (() -> Unit)? = null
    private var airRemotePalette: RemoteEditorPalette? = null
    private var airRemoteOwner: WorkbenchRemoteEdit? = null
    private var airRemoteRegistration: Any? = null
    private var airRemoteMotion: AirActionRemoteMotion? = null
    private var airRemoteContactActive by mutableStateOf(false)
    private var airRegisteredRemote: WorkbenchRemoteEdit? = null
    private var airRemoteChanges: Job? = null
    private var airObservedPage = 0u
    private val airModals = mutableSetOf<Any>()
    internal var airSetup by mutableStateOf(AirActionSetup()); private set
    internal var airCommandBusy by mutableStateOf(false); private set
    private val airDispatcher = CommandDispatcher(scope)
    internal val airActionRouter: AirActionRouter by lazy(LazyThreadSafetyMode.NONE) {
        AirActionRouter(airDispatcher, SystemClock::uptimeMillis, ::airBinding,
            { ticket, command -> airCommandAllowed(ticket, command) }, ::executeAirCommand,
            { message = "The action queue is full. Finish the current action, then try again." })
    }
    private var precisionClosed=false
    internal fun cancelPrecision(){if(!precisionClosed)precision.cancel()}
    private fun closePrecision(){precision.close();precisionClosed=true}
    private var synchronizingPrecision=false
    val precision:PrecisionController=PrecisionController(core,changed={if(!disposing&&!synchronizingPrecision)publish()})
    private var documentPress:Pair<Point,Point>?=null
    private var documentPressPage:UInt=0u
    private var documentPressRevision:ProjectInfo?=null
    private var documentPressEpoch:Long=0
    private var documentPressRegion=false
    private val canvasOwners=CanvasRetirementOwner()
    private data class ComposedBackgroundOwner(val project: WorkbenchProject, val epoch: Long,
        val documentId: String, val page: UInt?, val binding: WorkflowBinding)
    private var composedBackground: ComposedBackgroundOwner? = null
    private fun composedCurrent(doc: DocumentSnapshot): Boolean {
        val revision = doc.render.revision
        val owner = composedBackground ?: return false
        return owner.project === project && owner.epoch == epoch && owner.documentId == doc.documentId &&
            owner.page == doc.page && owner.binding == WorkflowBinding(revision.projectId, doc.documentId,
            revision.hostSeq, revision.stateHash) && background != null
    }
    private suspend fun preparedDrawing(items: List<RenderItem>): List<DrawObject> =
        prepareEditorObjects(core, items.filterNot { it.shape is Shape.Adjustment })
    private var giantRenderReady=false
    private var updatingGiant=false
    private val canvasRetirement:suspend()->Unit = {giantRenderReady=false;canvasOwners.retireAll()}
    internal var giantMode by mutableStateOf(false); private set
    internal var giantIssue by mutableStateOf<String?>(null); private set
    private val giantDraw = java.util.concurrent.atomic.AtomicReference<GiantVisibleFrame<Bitmap>?>(null)
    private val giantExpectedContent = java.util.concurrent.atomic.AtomicReference<GiantTileContent?>(null)
    private fun expectedGiantContent(doc: DocumentSnapshot): GiantTileContent =
        if (doc.hasToneAdjustments) GiantTileContent.Composed else GiantTileContent.Source
    private fun approvedAssumption(doc: DocumentSnapshot): Boolean {
        val format = originalFormat ?: return false
        val revision = doc.render.revision
        return colorAssumed && colorAssumedSourceAsset == format.sourceAssetId &&
            format.binding == WorkflowBinding(revision.projectId, doc.documentId, revision.hostSeq, revision.stateHash)
    }
    private val giantOwner = GiantAttachmentOwner<GiantAndroidCanvas>(
        attachment={val p=project;val doc=document;if(p!=null&&doc!=null&&!disposing)DocumentAttachment(p,doc,epoch)else null},
        create={image,resources,binding,attached,current->
            val snapshot = document?.takeIf { doc ->
                val revision = doc.render.revision
                WorkflowBinding(revision.projectId, doc.documentId, revision.hostSeq, revision.stateHash) == binding
            }
            // The factory consumes image ownership even if a replacement document
            // wins this race before it can allocate its first surface.
            GiantAndroidCanvas.create(scope,image,resources,binding,attached,
                { value, captured -> snapshot!=null && current(value,captured) },
                {canvasRetirement()},{publish()},
                content=snapshot?.let(::expectedGiantContent) ?: GiantTileContent.Source,
                assumeUntaggedSrgb=snapshot?.let(::approvedAssumption)==true)
        }, closeView={it.close()}, changed={giantDraw.set(null);publish()},
        diskCacheBytes=2uL*1024uL*1024uL*1024uL,ramCacheBytes=64uL*1024uL*1024uL,
        failedCreationOwner={ (it as? GiantAndroidCreationFailure)?.owner })
    internal fun giantSurfaceReady():Boolean = !giantMode || giantOwner.current?.surfaceReserved == true
    internal fun canCreateCanvas(hook:suspend()->Unit):Boolean = canvasOwners.canCreate(hook)&&giantSurfaceReady()
    internal fun canvasRendererRetired(hook:suspend()->Unit){canvasOwners.confirmed(hook)}
    internal fun canvasRendererRetiring(){giantRenderReady=false}
    internal fun giantRenderCompleted(frame:GiantVisibleFrame<Bitmap>,hook:suspend()->Unit){
        if(canvasOwners.canCreate(hook)&&giantFrameCurrent(frame))giantRenderReady=true
    }
    private fun giantInputReady(doc:DocumentSnapshot):Boolean = giantRenderReady&&giantFrame(doc)!=null
    internal fun giantFrameCurrent(frame:GiantVisibleFrame<Bitmap>):Boolean =
        giantDraw.get() === frame && giantExpectedContent.get() == frame.content
    private fun giantFrame(doc:DocumentSnapshot):GiantVisibleFrame<Bitmap>? {
        if(!giantMode)return null
        val view=giantOwner.current?:return null
        val revision=doc.render.revision
        return runCatching{view.matchingFrame(WorkflowBinding(revision.projectId,doc.documentId,revision.hostSeq,revision.stateHash),epoch,
            giantViewport(core,camera,precision.frame()),expectedGiantContent(doc))}.getOrNull()
    }
    private suspend fun detachGiant() {giantExpectedContent.set(null);giantDraw.set(null);giantOwner.detach();giantMode=false;giantIssue=null}
    private suspend fun loadGiantBackground(current:WorkbenchProject,doc:DocumentSnapshot) {
        giantExpectedContent.set(expectedGiantContent(doc));giantDraw.set(null)
        giantMode=true;background=null;composedBackground=null;giantRenderReady=false;giantOwner.invalidate()
        val revision = doc.render.revision
        val format = originalFormat?.takeIf { it.binding == WorkflowBinding(revision.projectId,doc.documentId,
            revision.hostSeq,revision.stateHash) }
        if(format==null){
            needsColorConsent=false;blockingMessage="The giant original's color and source information could not be checked. Reopen the project to retry."
            publish();return
        }
        if(format.untagged&&!approvedAssumption(doc)){
            needsColorConsent=true;blockingMessage="Tiled display requires an explicit sRGB assumption for this untagged source. The original remains unchanged."
            publish();return
        }
        needsColorConsent=false;blockingMessage=null;giantIssue=null
        canvasRetirement()
        try {
            val view=giantOwner.bind(DocumentAttachment(current,doc,epoch))?:return
            view.update(core,camera,precision.frame())
        } catch(cancel:CancellationException){throw cancel}
        catch(_:Exception){giantIssue="The original is preserved. Its verified tiled display is unavailable. Retry display or reopen the project."}
        publish()
    }
    internal fun retryGiantDisplay() {
        if(!giantMode||disposing||busy)return
        busy=true;scope.launch{try{loadBackground()}catch(error:Exception){report(error)}finally{busy=false;publish()}}
    }
    private val documentOwner:EditorDocuments=EditorDocuments(scope,core,
        { val p=project;val snapshot=document;if(p!=null&&snapshot!=null&&!disposing)DocumentAttachment(p,snapshot,epoch)else null },
        { !disposing&&assistedActions==null&&!workflows.blocksInput()&&!imageFrames.state.value.blocksInput&&!busy&&pending==0&&!ai.state.value.open&&!instructionEditor.state.value.busy&&!semanticEditor.state.value.busy&&!instructionEditor.hasUnsaved()&&instructionAdmission.allows(textDraft!=null,showBrush,showContext,blockingMessage!=null)&&gesture==null&&documentPress==null&&!eraserPointer&&!selectionPointer },
        { attached,page -> closeAssisted();mutations.withLock {
            if(project!==attached.project||epoch!=attached.epoch||!documentRevisionMatches(attached))throw DocumentFailure(DocumentFailureKind.Stale)
            refreshAdmission.invalidate();documentRefresh.invalidate()
            cancelTool();imageFrames.detach();workflows.detach();rotation.detach();detachGiant();erasers.detach();selections.detach();documentOwnerDiscardDraw()
            val next=(attached.project as WorkbenchDocumentsProject).selectDocumentPage(attached.snapshot.documentId,page)
            val drawing=preparedDrawing(next.render.items)
            if(project!==attached.project||epoch!=attached.epoch||disposing)throw DocumentFailure(DocumentFailureKind.Stale)
            document=next;info=next.render.revision;objects=drawing;background=null;composedBackground=null;selected=null;cameraFitted=false
            selections.bind(attached.project,next,epoch,refreshAdmission.capture().sequence,camera)
            erasers.bind(attached.project,next,epoch,refreshAdmission.capture().sequence,camera)
            instructionEditor.refresh();instructionFocus.revisionChanged();semanticEditor.refresh();fit();bindWorkflows();enterWorkflows()
        } }, { publish() },application,{canvasRetirement()})
    val documents:DocumentController get()=documentOwner.controller
    private fun documentRevisionMatches(attached:DocumentAttachment):Boolean = document?.render?.revision?.sameDocumentRevision(attached.snapshot.render.revision)==true
    private suspend fun documentOwnerDiscardDraw(){documentOwner.discardDraw()}
    private val repository = ProjectRepository(application, core)
    private val preferencesStore = EditorPreferencesStore(application)
    private val preferencesQueue = Channel<EditorPreferences>(Channel.CONFLATED)
    private val mutations = Mutex()
    private val refreshMutex = Mutex()
    private val refreshAdmission = RefreshAdmission()
    private val documentRefresh = DocumentRefreshAdmission()
    private var changesJob: Job? = null
    private var refreshJob: Job? = null
    private var refreshQueue: Channel<Unit>? = null
    val pairing = PairingController(application, scope, { repository.deviceId }, { message = it }, sessionFactory)
    val agentCapture = AgentCaptureObserver(scope, captureStatusFactory)
    val captureArrivals = CaptureArrivals(scope, captureArrivalQuery)
    private var link: ProjectLink? = null
    var remoteCapability by mutableStateOf<WorkbenchRemoteEdit?>(null); private set
    var remoteOpen by mutableStateOf(false); private set
    fun openRemoteComputer(){
        if(connection?.carrier!=SessionCarrier.QuicTether){message="Connect the selected USB tether route first.";return}
        try{interruptAirInput(clearBarrel = true);remoteCapability=remoteCapability?:remoteEdit(checkNotNull(link));remoteOpen=true;remoteCapability?.let(::observeAirRemote)}
        catch(_:Exception){message="Remote editing is unavailable on this authenticated link."}
    }
    fun remoteClosed(){
        val remote=airRemoteOwner;val palette=airRemotePalette;val registration=airRemoteRegistration
        if(remote!=null&&palette!=null&&registration!=null)detachAirRemotePalette(remote,palette,registration)
        interruptAirInput(clearBarrel = true);airRemoteChanges?.cancel();airRemoteChanges=null
        airRemoteMotion=null;airRemoteContactActive=false;airRemotePalette=null;airRemoteOwner=null;airRemoteRegistration=null;airRemoteFocused=false;remoteOpen=false;remoteCapability=null
    }
    private var linkChanges: Job? = null
    private var peerWorker: Job? = null
    private var connectionJob: Job? = null
    private var linkEpoch = 0L
    private var peerAdmission = 0L
    private var peerObjects = emptyList<DrawObject>()
    private var lastViewport: PeerViewport? = null
    var connection by mutableStateOf<SessionStatus?>(null); private set
    var followPeer by mutableStateOf(false); private set
    private var sliderGesture: String? = null
    private var sliderLink: ProjectLink? = null
    private var sliderSequence = 0u
    private var sliderRevision: ProjectInfo? = null
    val diagnostics = DiagnosticsController(application, scope)
    private var screenState by mutableStateOf(WorkbenchScreen.Projects)
    var screen: WorkbenchScreen
        get() = screenState
        private set(value) { if (screenState != value) interruptAirInput(clearBarrel = true); screenState = value }
    var preferences by mutableStateOf(EditorPreferences()); private set
    var tool by mutableStateOf(EditorTool.Pen); private set
    var projects by mutableStateOf<List<ProjectCard>>(emptyList()); private set
    // Native-operation admission must not wait for a Compose host or
    // global snapshot apply notifications. UI state remains observable.
    private val importReadiness = EditorReadiness()
    private var busyState by mutableStateOf(true)
    var busy: Boolean
        get() = busyState
        private set(value) { if (busyState != value) interruptAirInput(); busyState = value; importReadiness.busy(value) }
    var transferLabel by mutableStateOf<String?>(null); private set
    private var pendingState by mutableStateOf(0)
    var pending: Int
        get() = pendingState
        private set(value) { if (pendingState != value) interruptAirInput(); pendingState = value; importReadiness.pending(value) }
    var message by mutableStateOf<String?>(null); private set
    private var blockingMessageState by mutableStateOf<String?>(null)
    var blockingMessage: String?
        get() = blockingMessageState
        private set(value) { if (blockingMessageState != value) interruptAirInput(); blockingMessageState = value }
    var needsColorConsent by mutableStateOf(false); private set
    var originalFormatNotice by mutableStateOf<String?>(null); private set
    private var originalFormat by mutableStateOf<OriginalFormatInfo?>(null)
    var info by mutableStateOf<ProjectInfo?>(null); private set
    private var documentState by mutableStateOf<DocumentSnapshot?>(null)
    var document: DocumentSnapshot?
        get() = documentState
        private set(value) {
            if (documentState !== value) interruptAirInput(clearView = documentState?.documentId != value?.documentId ||
                documentState?.width != value?.width || documentState?.height != value?.height)
            if(documentState?.documentId!=value?.documentId || documentState?.page!=value?.page ||
                documentState?.render?.revision!=value?.render?.revision)invalidateAssisted()
            documentState = value
        }
    var selected by mutableStateOf<String?>(null); private set
    private var textDraftState by mutableStateOf<TextDraft?>(null)
    var textDraft: TextDraft?
        get() = textDraftState
        private set(value) { if (textDraftState != value) interruptAirInput(); textDraftState = value }
    private var showContextState by mutableStateOf(false)
    var showContext: Boolean
        get() = showContextState
        set(value) { if (showContextState != value) interruptAirInput(); showContextState = value }
    private var showBrushState by mutableStateOf(false)
    var showBrush: Boolean
        get() = showBrushState
        private set(value) { if (showBrushState != value) interruptAirInput(); showBrushState = value }
    var brushPreview by mutableStateOf<Path?>(null); private set
    var capabilities by mutableStateOf<StylusCapabilities?>(null); private set
    var scene by mutableStateOf<CanvasScene?>(null); private set
    var drawCallbacks by mutableStateOf(0L); private set
    var lastDrawCallbackMs by mutableStateOf(0L); private set
    var exportReady by mutableStateOf<ExportReady?>(null); private set
    var shareUri by mutableStateOf<Uri?>(null); private set
    var exportSelection by mutableStateOf(ExportSelection()); private set
    var exportPreflight by mutableStateOf<FilePreflight?>(null); private set
    var cameraCaptures by mutableStateOf<List<CameraCapture>>(emptyList()); private set
    private var checkedExport: ExportOptions? = null
    private var checkedExportIntent: ExportCallerIntent? = null
    private var pickerTicket: ExportTicket? = null
    private var project: WorkbenchProject? = null
        set(value) { if (field !== value) {interruptAirInput(clearView = true, clearBarrel = true);invalidateAssisted()}; field = value }
    private var transferJob: Job? = null
    private var incomingJob: Job? = null
    private var incomingProducer: Job? = null
    private var explicitImportCancel = false
    private var shareLease: MediaHandoff.Lease? = null
    private var disposing = false
    private var epoch = 0L
        set(value) { if (field != value) {interruptAirInput(clearView = true, clearBarrel = true);invalidateAssisted()}; field = value }
    private var sceneRevision = 0L
    private var background: Bitmap? = null
    private var objects = emptyList<DrawObject>()
    private var camera = Camera(Point(800.0, 600.0), 1.0, 0.0, 1.0, 1.0)
        set(value) { if (field != value) interruptAirInput(clearView = !applyingAirView); field = value }
    private var cameraFitted = false
    private var unsnappedRotation = 0.0
    private var density = 1f
    private var hoverPoint: Point? = null
    private var transformed = emptyMap<String, Transform>()
    private var erased = emptySet<String>()
    private var transientShape: DrawObject? = null
    private var gesture: ToolGesture? = null
    private var selectionPointer = false
    private var eraserPointer = false
    val selections = SelectionController(application, scope, core, {
        val current = project
        if (!disposing && current != null && document != null) refresh(current, current.info(), epoch)
    })
    val erasers = EraserController(core, scope, selections.interaction) { receipt ->
        val current = project
        if (!disposing && current != null && document != null) {
            selected?.let { id -> if (receipt != null) {
                if (id in receipt.replacements) selected = receipt.replacements[id]
                else if (id in receipt.removed) selected = null
            } }
            refresh(current, current.info(), epoch)
        }
    }
    private fun assistedCurrent(projectOwner: WorkbenchProject, source: AssistedSelectionSource): Boolean {
        val doc=document ?: return false
        val revision=doc.render.revision
        return !disposing && project === projectOwner && epoch == source.attachmentEpoch &&
            doc.page == source.page && source.document.width == doc.width && source.document.height == doc.height &&
            source.document.sourceBitDepth == doc.bitDepth &&
            source.binding.projectId == revision.projectId && source.binding.documentId == doc.documentId &&
            source.binding.hostSeq == revision.hostSeq && source.binding.stateHash == revision.stateHash &&
            assistedSource.matches(projectOwner,source)
    }
    private fun invalidateAssisted() { assistedSource.revoke();assistedCollecting=false;assistedTap=null;assistedActions?.invalidate() }
    private suspend fun closeAssisted() {
        invalidateAssisted();assistedPanelOpen=false
        val owner=assistedActions ?: return
        owner.close() // Keep this exact owner if native/runtime retirement fails.
        if(assistedActions===owner)assistedActions=null
        assistedPrompt=AssistedSelectionPrompt();assistedSubmittedPrompt=null;assistedNewObjectId=null
    }
    internal fun openAssistedSelection() {
        val owner=project ?: return
        val snapshot=document ?: return
        assistedActions?.let { actions -> if(assistedCurrent(owner,actions.source)){
            assistedCollecting=false;assistedPanelOpen=true;return
        } }
        if(assistedOpening)return
        if(disposing||!workflowIdle()||documents.state.value.descriptor!=null||blockingMessage!=null||
            originalFormat?.rawPreview==true){message="Assisted selection requires an available raster image preview.";return}
        val capturedEpoch=epoch
        assistedOpening=true
        cancelTool();selections.interaction.active(true)
        scope.launch {
            try {
                closeAssisted()
                val expected=snapshot.render.revision
                val selected=withTimeoutOrNull(5_000) { selections.state.first { state ->
                    state.document?.let { value -> value.binding.projectId==expected.projectId &&
                        value.binding.documentId==snapshot.documentId && value.binding.hostSeq==expected.hostSeq &&
                        value.binding.stateHash==expected.stateHash } == true }.document }
                if(selected==null){message="The selection source is still unavailable. Try again or use manual selection.";return@launch}
                val source=AssistedSelectionSource(selected,snapshot.page,capturedEpoch)
                if(disposing||project!==owner||epoch!=capturedEpoch||document?.render?.revision!=expected||document?.page!=snapshot.page)return@launch
                assistedSource.publish(owner,source)
                val backend=assistedPins?.let { liteRtAssistedSelectionBackend(getApplication(),assistedCatalog,owner,
                    {assistedSource.sourceFor(owner)},approvedAssumption(snapshot)) }
                val actions=AssistedSelectionInteraction(scope,source,owner.selections(),mutations,
                    metadata={val info=owner.info();WorkflowMetadata(newId(),repository.deviceId,
                        repository.instructionLamport(info.nextLamport),System.currentTimeMillis())},
                    isCurrent={candidate->assistedCurrent(owner,candidate)},
                    afterMutation={receipt->
                        if(project===owner&&epoch==capturedEpoch&&!disposing){refresh(owner,owner.info(),capturedEpoch)
                            receipt?.let{selections.interaction.useObject(it.snapshot.selection.objectId)}}
                    },pins=assistedPins,backend=backend)
                if(!assistedCurrent(owner,source)){actions.close();return@launch}
                assistedActions=actions;assistedPrompt=AssistedSelectionPrompt();assistedSubmittedPrompt=null;assistedNegative=false
                assistedNewObjectId=newId();assistedPanelOpen=true
            } catch(cancel:CancellationException){throw cancel}
            catch(error:Exception){assistedSource.revoke();report(error)}
            finally{assistedOpening=false}
        }
    }
    internal fun assistedPromptPolarity(negative:Boolean){assistedNegative=negative}
    internal fun clearAssistedPrompts(){assistedPrompt=AssistedSelectionPrompt();assistedSubmittedPrompt=null}
    internal fun proposeAssisted(){val prompt=assistedPrompt
        if(assistedActions?.prompt(prompt)==true)assistedSubmittedPrompt=prompt
    }
    internal fun dismissAssistedPanel(){assistedPanelOpen=false;assistedCollecting=assistedActions?.state?.value?.canPrompt==true
        if(!assistedCollecting)scope.launch { try{closeAssisted()}catch(error:Exception){report(error)} }
    }
    internal fun assistedManualSelection(objectId:String?){dismissAssistedPanel();assistedCollecting=false
        if(objectId==null)selections.interaction.newSelection() else selections.interaction.useObject(objectId)
        scope.launch { try{closeAssisted()}catch(error:Exception){report(error)} }
    }
    internal fun assistedTarget():SelectionTarget? {
        val selected=selections.state.value.selection?.takeIf { it.binding==assistedActions?.source?.binding }
        if(selected!=null&&selected.editable)return SelectionTarget.Existing(selected.selection)
        val layer=runCatching{selectedLayer()}.getOrNull()?:return null
        return SelectionTarget.New(assistedNewObjectId ?: return null,layer.id)
    }
    private val strokes = LinkedHashSet<StrokeRun>()
    private var previewJob: Job? = null
    private var previewHandle: WorkbenchStroke? = null
    private var colorAssumed = false
    private var colorAssumedSourceAsset: String? = null
    private var canvasSink: ((CanvasScene) -> Unit)? = null
    private var operationCount = 0
    private var eraserLast: Point? = null
    private var selectedStyleDraft by mutableStateOf<ObjectStyle?>(null)
    private val pendingPreviews = LinkedHashMap<String, DrawObject>()
    private val pendingTransforms = LinkedHashMap<String, Map<String, Transform>>()
    private val pendingErases = LinkedHashMap<String, Set<String>>()
    private val captureOwner = com.visualworkbench.android.capture.CaptureOwner<com.visualworkbench.android.capture.CaptureRuntime> { it.close() }
    val instructionEditor = InstructionEditor(scope, mutations,
        { val p = project; val d = document; if (p != null && d != null && !disposing) InstructionAttachment(p, d) else null },
        { newId() },
        { current -> val next = current.info(); WorkflowMetadata(newId(), repository.deviceId,
            repository.instructionLamport(next.nextLamport), System.currentTimeMillis()) },
        { receipt -> val current = project; if (current != null && !disposing) refresh(current, receipt.revision, epoch) },
        InstructionEntryMethod.PhoneKeyboard, localFocused = ::localInstructionFocused)
    val semanticEditor = SemanticEditor(scope, mutations,
        { val p = project; val d = document; if (p != null && d != null && !disposing) InstructionAttachment(p, d) else null },
        { !disposing && !busy && !ai.state.value.open && pending == 0 && gesture == null && strokes.isEmpty() &&
            !instructionEditor.hasUnsaved() && !instructionEditor.state.value.busy &&
            instructionAdmission.allows(textDraft != null, showBrush, showContext, blockingMessage != null) },
        { current -> val next = current.info(); WorkflowMetadata(newId(), repository.deviceId,
            repository.instructionLamport(next.nextLamport), System.currentTimeMillis()) },
        { receipt -> val current = project; if (current != null && !disposing) refresh(current, receipt.revision, epoch) },
        { placement -> instructionEditor.place(placement.point, placement.style, placement.bounds, placement.elementEids,
            placement.snapshotId, placement.binding) }, interactionChanged = ::instructionInteractionChanged)
    private val instructionAdmission = com.visualworkbench.android.instructions.InstructionFocusAdmission { instructionInteractionChanged() }
    val instructionFocus = InstructionFocusController(scope, instructionEditor,
        { val p = project; val d = document; if (p != null && d != null && !disposing) InstructionAttachment(p, d) else null },
        { !disposing && !busy && !semanticEditor.state.value.busy && pending == 0 && gesture == null && strokes.isEmpty() &&
            !ai.state.value.open && instructionAdmission.allows(textDraft != null, showBrush, showContext, blockingMessage != null) &&
            screen in setOf(WorkbenchScreen.Canvas, WorkbenchScreen.Instructions) },
        { id, peer -> selected = id; if (peer) screen = WorkbenchScreen.Instructions; publish() }, selectedObject = { selected })
    private fun localInstructionFocused(field: InstructionField) { instructionFocus.localField(field) }
    internal var captureAdmissionVersion by mutableStateOf(0L); private set
    fun instructionInteractionChanged() { interruptAirInput(); captureAdmissionVersion++; instructionFocus.interactionChanged() }
    internal fun instructionModal(owner: com.visualworkbench.android.instructions.InstructionFocusModal, active: Boolean) { instructionAdmission.modal(owner, active) }
    internal fun clearInstructionModals() { instructionAdmission.clear() }
    fun editInstruction() {
        if (document == null) { message = "Open a document first."; return }
        cancelTool(); screen = WorkbenchScreen.Instructions
        selected?.let(instructionEditor::focusObject) ?: instructionEditor.focusGlobal()
    }
    private var aiRetiring=false
    private fun aiAttachment(): AiAttachment? = if(aiRetiring||disposing)null else project?.let{p->document?.let{AiAttachment(p,it,epoch)}}
    private fun aiPoses():Map<String,Transform> = peerObjects.asSequence().filter{it.item.shape is Shape.Result}.take(9)
        .associate{it.item.objectId to it.item.transform}+pendingTransforms.values.fold(emptyMap<String,Transform>()){a,b->a+b}+transformed
    val aiService=AiServiceController(scope,application.noBackupFilesDir.absolutePath){path,initialize->
        AndroidProviderRuntime.initialize(application);createAiService(path,initialize)
    }
    val ai: AiEditorController<AiBitmap> = AiEditorController(scope,core,aiService,::aiAttachment,mutations,
        metadata={owned->val next=owned.project.info();WorkflowMetadata(newId(),repository.deviceId,repository.instructionLamport(next.nextLamport),System.currentTimeMillis())},
        refreshAfterMutation={owned->if(owned.sameProject(aiAttachment()))refresh(owned.project,owned.project.info(),epoch)},
        image=::aiReadBitmap,abandonImage=AiBitmap::abandon,
        canOperate={!disposing&&!busy&&pending==0&&!instructionEditor.state.value.busy&&!semanticEditor.state.value.busy})
    val aiResults=AiResultController(scope,core,::aiAttachment,{camera},::aiPoses,image=::aiReadBitmap,abandonImage=AiBitmap::abandon)
    fun aiKeyStore():AndroidProviderKeyStore = AndroidProviderRuntime.initialize(getApplication())
    private fun aiFrame()=aiResults.state.value.frame?.takeIf{it.matches(aiAttachment(),camera,aiPoses())}
    private fun aiResultReady()=document?.render?.items?.none{it.shape is Shape.Result}!=false||
        document?.let { composedCurrent(it) || giantFrame(it)?.content == GiantTileContent.Composed }==true||aiFrame()!=null
    fun openAi(){
        if(workflows.blocksInput()||imageFrames.state.value.blocksInput)return
        if(documents.state.value.descriptor!=null){message="AI image edits require a raster image document.";return}
        if(disposing||busy||pending!=0||document==null||textDraft!=null||showBrush||showContext||blockingMessage!=null)return
        if(instructionEditor.hasUnsaved()){message="Save or discard the instruction draft before opening AI review.";return}
        cancelTool();ai.open();instructionInteractionChanged()
    }
    fun hideAi(){ai.hide();instructionInteractionChanged()}
    private fun mayCloseAi():Boolean{
        if(!workflows.mayLeave())return false
        if(!ai.needsDecision())return true
        message="Save the paid Result or explicitly discard its review before replacing or closing this project."
        if(!instructionEditor.hasUnsaved()){ai.open();instructionInteractionChanged()}
        return false
    }
    private suspend fun retireAi(){
        aiRetiring=true
        try{if(disposing)ai.close()else ai.detach()}
        finally{try{if(disposing)aiResults.close()else aiResults.detach()}finally{if(disposing)aiService.close()}}
    }
    private fun workflowAttachment():EditorWorkflowAttachment? {
        val current=project;val doc=document
        return if(current!=null&&doc!=null&&!disposing)EditorWorkflowAttachment(current,doc,epoch,refreshAdmission.capture().sequence,repository.deviceId)else null
    }
    private fun workflowIdle():Boolean = frameIdle() && !imageFrames.state.value.blocksInput
    private fun frameIdle():Boolean = !disposing&&assistedActions==null&&!busy&&pending==0&&operationCount==0&&screen==WorkbenchScreen.Canvas&&
        !navigationContactActive&&!rotation.state.value.active&&!rotationUnavailable()&&gesture==null&&strokes.isEmpty()&&documentPress==null&&!selectionPointer&&!eraserPointer&&
        sliderGesture==null&&selectedStyleDraft==null&&!showBrush&&!showContext&&textDraft==null&&blockingMessage==null&&
        !ai.state.value.open&&!ai.needsDecision()&&!instructionEditor.state.value.busy&&!instructionEditor.hasUnsaved()&&
        !semanticEditor.state.value.busy&&!selections.state.value.busy&&!erasers.state.value.busy&&!documents.state.value.busy&&
        !remoteOpen&&transferJob==null&&incomingJob==null&&incomingProducer==null&&connectionJob?.isActive!=true&&
        captureOpenWork==null&&previewJob?.isActive!=true&&previewHandle==null&&pickerTicket==null&&shareLease==null&&
        instructionAdmission.allows(false,false,false,false)
    internal fun workflowId():String=newId()
    internal val workflows=EditorWorkflows(scope,mutations,::workflowAttachment,::workflowIdle,{cancelTool();followPeer=false},
        metadata={current->val next=current.info();WorkflowMetadata(newId(),repository.deviceId,
            repository.instructionLamport(next.nextLamport),System.currentTimeMillis())},
        newId=::newId,camera={camera},applyCamera={value->invalidateViewCheck();followPeer=false;camera=value;cameraFitted=true;unsnappedRotation=value.rotation;publish()},
        updated={a,revision->if(project===a.project&&epoch==a.epoch&&!disposing){repository.observeLamport(revision.nextLamport);refresh(a.project,revision,a.epoch)}},
        committed={a,receipt->if(project===a.project&&epoch==a.epoch&&!disposing){repository.observeLamport(receipt.revision.nextLamport);refresh(a.project,receipt.revision,a.epoch)}},
        adoptDocument=::adoptWorkflowDocument,report={message=it},
        localViews=::localViewWorkflows,documentControls=::documentControlWorkflows,reviews=::offlineReviews,toneControls=::toneControlWorkflows)
    internal val imageFrames = ImageFramesInteraction(scope, mutations, ::newId,
        { current -> val next = current.info(); WorkflowMetadata(newId(), repository.deviceId,
            repository.instructionLamport(next.nextLamport), System.currentTimeMillis()) },
        { frameIdle() && !workflows.blocksInput() },
        { receipt -> val current = project; if (current != null && !disposing && receipt.revision.projectId == info?.projectId) {
            repository.observeLamport(receipt.revision.nextLamport); refresh(current, receipt.revision, epoch)
        } }, ::imageFrameWorkflows)
    internal val frameControlsAvailable: Boolean get() = document?.let { !giantDisplayRequired(it) } == true &&
        documents.state.value.descriptor == null && originalFormat != null && originalFormat?.rawPreview != true
    internal val toneControlsAvailable: Boolean get() = document != null && documents.state.value.descriptor == null &&
        originalFormat != null && originalFormat?.rawPreview != true
    internal val rawPreview: Boolean get() = originalFormat?.rawPreview == true
    internal fun framePanel(active: Boolean) {
        if (active && !imageFrames.state.value.blocksInput && (!frameControlsAvailable || !workflowIdle())) {
            message = if (originalFormat?.rawPreview == true) "RAW sensor originals are not editable through the embedded preview."
                else "Finish the current edit or dialog before opening the output frame."
            return
        }
        if (!imageFrames.active(active)) message = "Apply or discard private frame values before closing."
    }
    private var rotationGesture:ObjectRotationGesture?=null
    private var rotationDisplay:ObjectRotationHandle?=null
    private var rotationPhysical:Transform?=null
    internal var rotationSnap15 by mutableStateOf(false); private set
    internal fun rotationSnap(value:Boolean){if(!disposing&&!workflows.blocksInput()&&!imageFrames.state.value.blocksInput)rotationSnap15=value}
    private fun rotationUnavailable():Boolean=rotation.state.value.busy||rotation.state.value.failure==ObjectRotationFailure.Unconfirmed
    private fun rotationAdmission():Boolean = !disposing&&!busy&&pending==0&&operationCount==0&&!workflows.blocksInput()&&!imageFrames.state.value.blocksInput&&
        !instructionEditor.state.value.busy&&!semanticEditor.state.value.busy&&!ai.state.value.open&&
        instructionAdmission.allows(textDraft!=null,showBrush,showContext,blockingMessage!=null)
    internal val rotation=ObjectRotationInteraction(scope,mutations,::newId,::objectRotationTransforms,
        current={val current=project;val doc=document;if(current!=null&&doc!=null&&!disposing)ObjectRotationAttachment(current,doc,epoch,camera,setOfNotNull(selected))else null},
        mayMutate=::rotationAdmission,
        commit={request->
            val doc=document
            if(project!==request.project||doc==null||doc.objectRotationFrame(epoch,camera)!=request.frame)false
            else{
                repository.observeLamport(request.project.info().nextLamport)
                val options=editOptions(request.commands.size).copy(gestureId=request.gestureId,
                    expectedHostSeq=request.frame.binding.hostSeq,expectedStateHash=request.frame.binding.stateHash)
                val revision=request.project.edit(options,request.commands)
                repository.observeLamport(revision.nextLamport);refresh(request.project,revision,epoch);true
            }
        })
    private fun rotationHandle():ObjectRotationHandle? {
        val doc=document?:return null
        if(tool!=EditorTool.Select||workflows.blocksInput()||imageFrames.state.value.blocksInput||blockingMessage!=null)return null
        val base=objectRotationHandle(doc,setOfNotNull(selected),camera,28.0*density)?:return null
        return rotationDisplay?:base
    }
    private fun beginRotation(point:Point,raw:Point,physical:Transform):Boolean {
        val handle=rotationHandle()?:return false
        if(!handle.hit(physical,raw,24.0*density))return false
        val token=rotation.begin(handle.pivot,point)?:return false
        rotationGesture=token;rotationDisplay=handle.copy(point=point);rotationPhysical=physical;publish();return true
    }
    private fun bindWorkflows(){rotation.reconciled();info?.let{repository.observeLamport(it.nextLamport)};workflowAttachment()?.let(workflows::bind)
        val current=project;val doc=document
        if(current!=null&&doc!=null&&!disposing)imageFrames.bind(current,doc,epoch,refreshAdmission.capture().sequence)
    }
    private suspend fun enterWorkflows(){workflowAttachment()?.let{workflows.enter(it)}}
    private suspend fun adoptWorkflowDocument(a:EditorWorkflowAttachment,target:DocumentSnapshot){
        if(project!==a.project||epoch!=a.epoch||document?.documentId!=a.document.documentId||document?.page!=a.document.page||document?.render?.revision?.sameDocumentRevision(a.document.render.revision)!=true||disposing)
            throw WorkflowFailure(WorkflowFailureKind.Stale)
        val drawing=preparedDrawing(target.render.items)
        cancelTool();followPeer=false
        // Keep the authenticated ProjectLink and native project/history. Only
        // subscriptions carrying the old attachment token retire here.
        suspend fun retireObservers(){
            changesJob?.cancelAndJoin();changesJob=null;refreshQueue?.close();refreshQueue=null
            refreshJob?.cancelAndJoin();refreshJob=null
            linkChanges?.cancelAndJoin();linkChanges=null;peerWorker?.cancelAndJoin();peerWorker=null
        }
        var metadataRetired=false
        withDocumentObserversRetired(
            retire={retireObservers()},
            restore={if(!disposing&&project===a.project){
                try {
                    if(metadataRetired){instructionEditor.resume();semanticEditor.resumeAfterReplacement();instructionEditor.refresh();instructionFocus.revisionChanged()}
                } finally {
                    observeProject(a.project,epoch);refreshQueue?.trySend(Unit)
                    link?.let{observeLink(it,a.project,epoch,linkEpoch,reattachAgent=false);instructionFocus.attach(it)}
                }
            }},
        ) {
            refreshMutex.withLock {
                imageFrames.detach();rotation.detach();semanticEditor.detach();instructionEditor.detach();instructionFocus.detach()
                metadataRetired=true
                erasers.detach();selections.detach();retireAi();detachGiant();documentOwner.detach()
                if(disposing||project!==a.project||epoch!=a.epoch)throw WorkflowFailure(WorkflowFailureKind.Stale)
                val actual=a.project.info();val expected=target.render.revision
                if(actual.projectId!=expected.projectId||actual.hostSeq!=expected.hostSeq||actual.stateHash!=expected.stateHash||target.documentId !in actual.documentIds)
                    throw WorkflowFailure(WorkflowFailureKind.Stale)
                invalidateExport();clearPeerObjects();captureArrivals.detach();captureNavigationGeneration++
                epoch++;refreshAdmission.reset();documentRefresh.invalidate();aiRetiring=false;colorAssumed=false;colorAssumedSourceAsset=null
                if(!precisionClosed)precision.reset()
                document=target;info=target.render.revision;objects=drawing;background=null;composedBackground=null;scene=null;selected=null
                transientShape=null;transformed=emptyMap();erased=emptySet();hoverPoint=null;selectedStyleDraft=null
                pendingPreviews.clear();pendingTransforms.clear();pendingErases.clear()
                blockingMessage=null;needsColorConsent=false;originalFormatNotice=null;originalFormat=null;cameraFitted=false;lastViewport=null
                instructionEditor.resume();semanticEditor.resumeAfterReplacement();metadataRetired=false
                selections.bind(a.project,target,epoch,refreshAdmission.capture().sequence,camera)
                erasers.bind(a.project,target,epoch,refreshAdmission.capture().sequence,camera)
                instructionEditor.refresh();instructionFocus.revisionChanged();semanticEditor.refresh();documentOwner.bind(a.project,target)
                fit();bindWorkflows();refreshOriginalFormatNotice();loadBackground()
            }
        }
    }
    private val preferencesWriter: Job

    init {
        airInitialized = true
        scope.launch { rotation.state.collect{value->
            if(!value.active&&!value.busy){rotationDisplay=null;rotationPhysical=null}
            publish()
            if(value.failure==ObjectRotationFailure.Unconfirmed&&!disposing){
                val current=project;val token=epoch
                if(current!=null)scope.launch{mutations.withLock{
                    if(current===project&&token==epoch&&!disposing)try{refresh(current,current.info(),token)}
                    catch(error:Exception){report(error)}
                }}
            }
        }}
        scope.launch { aiResults.state.collect { publish() } }
        preferencesWriter = scope.launch {
            for (settings in preferencesQueue) try { preferencesStore.write(settings) }
            catch (_: Exception) { message = "Settings could not be saved. Try changing the setting again." }
        }
        scope.launch {
            try {
                repository.initialize(); preferences = preferencesStore.read(); configureAirActions(); refreshProjects(); refreshCaptures()
                if (disposing || !kotlinx.coroutines.currentCoroutineContext().isActive) return@launch
                val owner = com.visualworkbench.android.capture.CaptureRuntime(core, repository.deviceId,
                    repository::checkedPath, { repository.lamport() }) { captured, warning ->
                    // This callback consumes the delivered handle. The saved
                    // project can be opened from Projects without stealing focus
                    // from the application whose screen the owner captured.
                    withContext(NonCancellable) { captured.close() }
                    if (!disposing) { refreshProjects(); message = warning ?: "Capture saved in Projects."; publish() }
                }
                captureOwner.install(owner, { !disposing }) { candidate ->
                    com.visualworkbench.android.capture.CaptureRuntime.installed.compareAndSet(null, candidate)
                }
            }
            catch (error: Exception) { report(error) }
            finally { busy = false }
        }
    }

    private var canvasHook:(suspend()->Unit)?=null
    fun attachCanvas(sink:(CanvasScene)->Unit,retire:suspend()->Unit = {},ready:()->Unit = {}) {
        canvasOwners.attach(retire,ready);canvasHook=retire;canvasSink=sink;giantRenderReady=false;scene?.let { sink(renderScene(it)) }
    }
    fun detachCanvas(retire:(suspend()->Unit)?=null) {
        val hook=retire?:canvasHook
        if(hook==null||canvasOwners.detach(hook)){canvasSink=null;canvasHook=null;giantRenderReady=false}
        cancelTool();hoverPoint=null
    }
    fun dismissMessage() { message = null }
    fun transferMessage(value: String) { message = value }
    internal fun attachAirActivity(owner: Any, receive: (KeyEvent) -> Boolean, schedule: () -> Unit) {
        interruptAirInput(clearBarrel = true); airActivity = owner; airResumed = false; airActivityFocused = false
        airKeyReceiver = receive; airSchedule = schedule; configureAirActions()
    }
    internal fun airActivityCurrent(owner: Any): Boolean = airActivity === owner
    internal fun airActivityKey(owner: Any, event: KeyEvent): Boolean =
        airActivity === owner && airActivityFocused && !remoteOpen && airKey(event)
    internal fun airActivityResumed(owner: Any, resumed: Boolean) {
        if (airActivity !== owner || airResumed == resumed) return
        if (!resumed) resetAirRemoteMotion()
        interruptAirInput(clearBarrel = true); airResumed = resumed
    }
    internal fun airActivityFocus(owner: Any, focused: Boolean) {
        if (airActivity !== owner || airActivityFocused == focused) return
        interruptAirInput(clearBarrel = true); airActivityFocused = focused
    }
    internal fun detachAirActivity(owner: Any) {
        if (airActivity !== owner) return
        resetAirRemoteMotion()
        interruptAirInput(clearBarrel = true); airActivity = null; airResumed = false; airActivityFocused = false
        airKeyReceiver = null; airSchedule = null
    }
    private fun airKey(event: KeyEvent): Boolean {
        val received = airKeyReceiver?.invoke(event) == true
        if (received && !event.isCanceled) airSetup = airSetup.copy(keyDeliveryObserved = true)
        return received
    }
    internal fun configureAirSetup(value: AirActionSetup) {
        interruptAirInput(); airSetup = value; configureAirActions()
    }
    internal fun remapAirAction(gesture: AirActionGesture, command: AirActionCommand?) {
        settings(preferences.copy(airActions = preferences.airActions.remap(gesture, command)))
    }
    private fun configureAirActions() {
        if (!airInitialized) return
        airActionRouter.configure(preferences.airActions, airSetup.routingReady)
        airBarrel.restore(airActionRouter, SystemClock.uptimeMillis())
        airSchedule?.invoke()
    }
    internal fun interruptAirInput(clearView: Boolean = false, clearBarrel: Boolean = false) {
        airEpoch.incrementAndGet()
        if (clearView) airViewRestore.clear()
        if (clearBarrel) airBarrel.clear()
        if (airInitialized) {
            airActionRouter.interrupt(); airBarrel.restore(airActionRouter, SystemClock.uptimeMillis()); airSchedule?.invoke()
        }
    }
    internal fun observeAirBarrel(packet: PointerPacket) {
        if (packet.canceled || !precisionInputAllowed()) { interruptAirInput(clearBarrel = true); return }
        val sample = packet.frames.lastOrNull()?.pointers?.firstOrNull {
            it.tool == PointerTool.Pen || it.tool == PointerTool.Eraser
        } ?: return
        val down = sample.buttons and MotionEvent.BUTTON_STYLUS_PRIMARY != 0
        observeAirBarrelState(down, sample.timeMs)
    }
    private fun observeAirBarrelState(down: Boolean, timeMs: Long) {
        airBarrel.observe(down, timeMs)
        airActionRouter.barrel(down, timeMs)
        airSchedule?.invoke()
    }
    private fun airBinding(): AirActionBinding? {
        if (disposing || airActivity == null || !airResumed) return null
        if (airModals.isNotEmpty()) return AirActionBinding(airEpoch.get(), AirActionContext.Dialog)
        val context = if (remoteOpen) {
            if (!airRemoteFocused || airRemoteOwner !== remoteCapability || airRemotePalette == null) return null
            AirActionContext.RemoteApp
        } else {
            if (!airActivityFocused) return null
            if (screen == WorkbenchScreen.Canvas) AirActionContext.Canvas else AirActionContext.Dialog
        }
        return AirActionBinding(airEpoch.get(), context)
    }
    internal fun airCommandEnabled(command: AirActionCommand): Boolean =
        airBinding()?.let { airCommandAllowed(it, command) } == true
    private fun airCommandAllowed(ticket: AirActionBinding, command: AirActionCommand, ownCommand: Boolean = false): Boolean {
        if (ticket != airBinding() || (!ownCommand && airCommandBusy)) return false
        return when (ticket.context) {
            AirActionContext.Canvas -> workflowIdle() && precisionInputAllowed() && !precision.state.value.latched &&
                camera.viewportWidth > 1 && camera.viewportHeight > 1 && precision.baseCamera() == camera &&
                when (command) {
                    AirActionCommand.Undo -> info?.canUndo == true
                    AirActionCommand.Redo -> info?.canRedo == true
                    AirActionCommand.LensMagnificationUp -> precision.state.value.magnification < 8
                    AirActionCommand.LensMagnificationDown -> precision.state.value.magnification > 2
                    else -> true
                }
            AirActionContext.RemoteApp -> {
                val action = remoteAirAction(command) ?: return false
                val palette = airRemotePalette ?: return false
                val target = remoteCapability?.target?.value ?: return false
                !airRemoteContactActive && !busy && pending == 0 && !workflows.blocksInput() && !rotationUnavailable() &&
                    !instructionEditor.state.value.busy && !instructionEditor.hasUnsaved() && !semanticEditor.state.value.busy &&
                    !ai.state.value.open && !ai.needsDecision() && !documents.state.value.busy &&
                    !selections.state.value.busy && !erasers.state.value.busy && !rotation.state.value.active &&
                    instructionAdmission.allows(textDraft != null, showBrush, showContext, blockingMessage != null) &&
                    remoteCapability?.state?.value?.status == RemoteEditStatus.Controlling &&
                    target.identity.editor == RemoteEditorKind.Paint && !palette.state.value.closed && !palette.state.value.busy &&
                    palette.state.value.target == target && remoteEditorActionEnabled(target, action)
            }
            AirActionContext.Dialog -> false
        }
    }
    internal fun airModal(owner: Any, active: Boolean) {
        val changed = if (active) airModals.add(owner) else airModals.remove(owner)
        if (changed) interruptAirInput()
    }
    internal fun airControl(command: AirActionCommand) {
        val ticket = airBinding() ?: return
        if (!airCommandAllowed(ticket, command)) return
        val time = SystemClock.uptimeMillis()
        if (!airDispatcher.offer {
            if (SystemClock.uptimeMillis() - time in 0L..AirActionRouter.MaximumPressMs && airCommandAllowed(ticket, command))
                executeAirCommand(ticket, command)
        }) message = "The action queue is full. Finish the current action, then try again."
    }
    private suspend fun executeAirCommand(ticket: AirActionBinding, command: AirActionCommand) {
        if (!airCommandAllowed(ticket, command)) return
        airCommandBusy = true
        try {
            if (ticket.context == AirActionContext.RemoteApp) {
                val palette = airRemotePalette ?: return
                val action = remoteAirAction(command) ?: return
                if (palette.command(action)) palette.state.first { !it.busy }
                else message = "The current PC command was refused. Check its grant and canvas, then try again."
            } else when (command) {
                AirActionCommand.Undo, AirActionCommand.Redo -> mutations.withLock {
                    // Recheck after lock acquisition before creating metadata or
                    // crossing the native commit point. This call is awaited.
                    if (!airCommandAllowed(ticket, command, ownCommand = true)) return@withLock
                    cancelTool()
                    val current = project ?: return@withLock
                    val token = epoch; val options = editOptions(512)
                    invalidateExport(); pending++
                    try { commitUndoRedo(current, token, options, command == AirActionCommand.Redo) }
                    finally { pending-- }
                }
                AirActionCommand.TogglePrecisionLens -> precision.toggle()
                AirActionCommand.FitRestoreView -> airFitRestore()
                AirActionCommand.PreviousTool, AirActionCommand.NextTool -> {
                    val tools = EditorTool.entries
                    val selection = selections.state.value
                    val active = if (erasers.state.value.active || (selection.active && selection.tool == SelectionTool.MaskEraser)) EditorTool.Eraser else tool
                    val offset = if (command == AirActionCommand.PreviousTool) -1 else 1
                    chooseTool(tools[(tools.indexOf(active) + offset + tools.size) % tools.size])
                }
                AirActionCommand.LensMagnificationUp, AirActionCommand.LensMagnificationDown -> {
                    val levels = listOf(2, 4, 8); val index = levels.indexOf(precision.state.value.magnification)
                    precision.magnification(levels[(index + if (command == AirActionCommand.LensMagnificationUp) 1 else -1).coerceIn(0, 2)])
                }
            }
        } catch (cancel: CancellationException) { throw cancel }
        catch (error: Exception) { report(error) }
        finally { airCommandBusy = false }
    }
    private fun airFitRestore() {
        val doc = document ?: return
        val owner = AirActionViewOwner(epoch, doc.documentId, documents.state.value.page, camera.viewportWidth, camera.viewportHeight)
        val before = camera
        val restore = airViewRestore.take(owner, before)
        applyingAirView = true
        try {
            if (restore == null) { fit(); airViewRestore.remember(owner, before, camera) }
            else {
                cancelTool(); invalidateViewCheck(); followPeer = false
                camera = restore; cameraFitted = true; unsnappedRotation = restore.rotation; publish()
            }
        } finally { applyingAirView = false }
    }
    internal fun attachAirRemotePalette(remote: WorkbenchRemoteEdit, palette: RemoteEditorPalette, registration: Any) {
        if (remoteCapability !== remote) return
        val previousRemote = airRemoteOwner; val previousPalette = airRemotePalette
        if (previousRemote != null && previousPalette != null) airRemoteMotion?.detach(previousRemote, previousPalette)
        interruptAirInput(clearBarrel = true); airRemoteOwner = remote; airRemotePalette = palette; airRemoteFocused = false
        airRemoteRegistration = registration
        airRemoteContactActive = false
        airRemoteMotion = AirActionRemoteMotion(remote, palette,
            { clear -> interruptAirInput(clearBarrel = clear) }, ::observeAirBarrelState)
    }
    internal fun detachAirRemotePalette(remote: WorkbenchRemoteEdit, palette: RemoteEditorPalette, registration: Any) {
        if (airRemoteOwner !== remote || airRemotePalette !== palette || airRemoteRegistration !== registration) return
        airRemoteMotion?.detach(remote, palette)
        interruptAirInput(clearBarrel = true); airRemoteMotion = null; airRemoteContactActive = false
        airRemoteOwner = null; airRemotePalette = null; airRemoteRegistration = null; airRemoteFocused = false
    }
    internal fun airRemoteFocus(remote: WorkbenchRemoteEdit, palette: RemoteEditorPalette, registration: Any, focused: Boolean) {
        if (airRemoteOwner !== remote || airRemotePalette !== palette || airRemoteRegistration !== registration) return
        val observer = airRemoteMotion
        if (observer?.focus(remote, palette, focused) == true) airRemoteContactActive = observer.contactActive
        if (airRemoteFocused != focused) { interruptAirInput(clearBarrel = true); airRemoteFocused = focused }
    }
    internal fun airRemoteKey(remote: WorkbenchRemoteEdit, palette: RemoteEditorPalette, registration: Any, event: KeyEvent): Boolean =
        airRemoteOwner === remote && airRemotePalette === palette && airRemoteRegistration === registration && airKey(event)
    internal fun observeAirRemoteMotion(remote: WorkbenchRemoteEdit, palette: RemoteEditorPalette, registration: Any, event: MotionEvent) {
        if (airRemoteOwner !== remote || airRemotePalette !== palette || airRemoteRegistration !== registration || !airRemoteFocused) return
        val observer = airRemoteMotion ?: return
        observer.observe(remote, palette, airRemoteMotionFrame(event))
        airRemoteContactActive = observer.contactActive
    }
    internal fun airRemoteMotionFailed(remote: WorkbenchRemoteEdit, palette: RemoteEditorPalette, registration: Any) {
        if (airRemoteOwner !== remote || airRemotePalette !== palette || airRemoteRegistration !== registration) return
        // Mark blocked before optional observer/timer work. Even an observer
        // failure cannot leave an old offer eligible after the motion delegate.
        airRemoteContactActive = true
        interruptAirInput(clearBarrel = true)
    }
    private fun resetAirRemoteMotion() {
        val remote = airRemoteOwner ?: return; val palette = airRemotePalette ?: return
        airRemoteMotion?.reset(remote, palette); airRemoteContactActive = false
    }
    private fun observeAirRemote(remote: WorkbenchRemoteEdit) {
        if (airRegisteredRemote !== remote) {
            airRegisteredRemote = remote
            remote.registerConsumer(object : RemoteConsumerOwner {
                override fun invalidate() {
                    // Native invalidation may precede a Main-dispatcher state
                    // publication. Retire the ticket synchronously on any thread.
                    airEpoch.incrementAndGet()
                    scope.launch { interruptAirInput() }
                }
                override suspend fun retire(): Boolean { invalidate(); return true }
            })
        }
        airRemoteChanges?.cancel()
        airRemoteChanges = scope.launch {
            var observed: List<Any?>? = null
            remote.target.collect { target ->
                // A newer timestamp for the same proof is not a new owner.
                val next = listOf(target?.binding, target?.identity, target?.verifiedActions,
                    target?.grantActive, target?.pauseReason, target?.commandBusy,
                    target?.focusProof?.runtimeIdHash, target?.focusProof?.canvasRectHost,
                    target?.selectedCanvasProof?.runtimeIdHash, target?.selectedCanvasProof?.canvasRectHost)
                if (next != observed) { observed = next; interruptAirInput() }
            }
        }
    }
    fun configureExport(value: ExportSelection) { if (!value.encoding.exportAvailable) { message = "This format is unavailable for an identity-preserving export."; return }; if (exportSelection != value) { exportSelection = value; invalidateExport() } }
    private fun invalidateExport() { exportReady = null; exportPreflight = null; checkedExport = null; checkedExportIntent = null }
    private fun invalidateViewCheck() {
        if (!applyingAirView) airViewRestore.clear()
        if (exportSelection.viewOnly) invalidateExport()
    }
    suspend fun cleanupTransfer(action: suspend () -> Unit) { cleanupHandoff(action) { message = "Temporary image cleanup could not finish. Retained files have not been deleted; you can retry or discard camera captures from Projects." } }
    private suspend fun refreshCaptures() {
        try { cameraCaptures = withContext(Dispatchers.IO) { MediaHandoff(getApplication()).captures().map { CameraCapture(it.token, it.file.length()) } } }
        catch (_: Exception) { message = "Retained camera captures could not be listed. They remain on this phone." }
    }
    fun importCamera(lease: MediaHandoff.Lease) = receiveImage(lease.uri) { outcome ->
        val handoff = MediaHandoff(getApplication())
        if (outcome != ImportOutcome.Retained) handoff.remove(lease, camera = true)
    }
    fun retryCamera(token: String) {
        scope.launch {
            try { importCamera(withContext(Dispatchers.IO) { MediaHandoff(getApplication()).resolve(token, true) }) }
            catch (error: Exception) { report(error) }
        }
    }
    fun discardCamera(token: String) {
        if (busy || incomingJob?.isActive == true) return
        scope.launch { cleanupTransfer { val h = MediaHandoff(getApplication()); h.remove(h.resolve(token, true), true) }; refreshCaptures() }
    }
    fun saveCamera(token: String, uri: Uri) {
        if (uri.scheme != "content") { message = "The destination must be a document provider."; return }
        if (busy || incomingJob?.isActive == true) { scope.launch { cleanupTransfer { repository.discardDestination(uri) } }; return }
        busy = true
        val job = scope.launch {
            var copying = false
            try {
                val lease = withContext(Dispatchers.IO) { MediaHandoff(getApplication()).resolve(token, true) }
                copying = true
                repository.saveCamera(lease.file, uri) { transferLabel = it }
                message = "Camera original saved. The retained capture remains available until you discard it."
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { report(error) }
            finally { if (!copying) cleanupTransfer { repository.discardDestination(uri) }; busy = false; transferLabel = null; transferJob = null }
        }
        transferJob = job.takeIf { it.isActive }
    }
    fun consumeShare(): Uri? = shareUri.also { shareUri = null; shareLease = null }
    fun shareLaunchFailed(lease: Uri) {
        scope.launch {
            cleanupTransfer {
                val token = lease.pathSegments.getOrNull(1) ?: return@cleanupTransfer
                val handoff = MediaHandoff(getApplication())
                handoff.remove(handoff.resolve(token, camera = false), camera = false)
            }
        }
        message = "No app could receive this PNG. Try saving it to Files."
    }
    fun settings(value: EditorPreferences) {
        preferences = value; configureAirActions(); preferencesQueue.trySend(value); publish()
    }
    fun chooseTool(value: EditorTool) {
        if (value == EditorTool.Eraser) {
            chooseEraser(if (erasers.state.value.mode == EraserMode.Stroke) EraserChoice.Stroke else EraserChoice.Object); return
        }
        cancelTool(); erasers.tools.drawing(); tool = value; showContext = false; hoverPoint = null; publish()
    }
    fun chooseEraser(value: EraserChoice) { cancelTool(); erasers.tools.choose(value, selected); showContext = false; hoverPoint = null; publish() }
    fun stopErasing() { cancelTool(); erasers.tools.drawing(); publish() }
    fun displayedBrush(): BrushPreference {
        val style = selectedStyleDraft ?: if (tool == EditorTool.Select) objects.firstOrNull { it.item.objectId == selected }?.item?.style else null
        return if (style == null) preferences.brush(tool) else preferences.brush(tool).copy(width = style.width.coerceIn(.25, 256.0), colorArgb = EditorDrawing.argb(style.rgba).toLong() and 0xffffffffL)
    }
    fun brush(value: BrushPreference) {
        val item = if (tool == EditorTool.Select) objects.firstOrNull { it.item.objectId == selected }?.item else null
        if (item != null) {
            val style = item.style.copy(width = value.width, rgba = EditorDrawing.rgba(value.colorArgb))
            if (showBrush) {
                selectedStyleDraft = style
                sliderGesture?.let { id -> sendObject(sliderLink, ObjectPreview(id, checkNotNull(document).documentId, item.objectId,
                    ++sliderSequence, PreviewKind.Slider, item.transform, style)) }
            } else edit(listOf(EditCommand.SetStyle(item.objectId, style)))
        } else settings(preferences.withBrush(tool, value))
        if (showBrush) previewBrush()
    }
    fun brushDialog(show: Boolean) {
        if (!show && selectedStyleDraft != null) {
            val item = objects.firstOrNull { it.item.objectId == selected }?.item
            val style = selectedStyleDraft
            if (item != null && style != null && item.style != style) edit(listOf(EditCommand.SetStyle(item.objectId, style)), gestureId = sliderGesture, previewLink = sliderLink, expected = sliderRevision)
            else finishRemotePreview(sliderLink, sliderGesture, true)
            selectedStyleDraft = null
        }
        if (!show) { sliderGesture = null; sliderLink = null; sliderRevision = null }
        showBrush = show
        if (show) {
            cancelTool(); sliderGesture = if (tool == EditorTool.Select && selected != null) newId() else null
            sliderLink = link; sliderSequence = 0u; sliderRevision = info; previewBrush()
        } else { previewJob?.cancel(); cancelNative(previewHandle); previewHandle = null }
    }
    fun navigate(next: WorkbenchScreen) {
        if(workflows.blocksInput()||imageFrames.state.value.blocksInput){message="Close the current controls or finish their operation first.";return}
        captureNavigationGeneration++ // Explicit navigation invalidates any staged arrival before suspension.
        if(next==WorkbenchScreen.Projects&&!mayCloseAi())return
        if (next == WorkbenchScreen.Projects && instructionEditor.hasUnsaved()) {
            message = "Save or discard the instruction draft before closing this project."; screen = WorkbenchScreen.Instructions; return
        }
        if (transferJob != null) { message = "Finish or cancel the image transfer before leaving this screen."; return }
        cancelTool(); diagnostics.background(); showContext = false
        if (screen == WorkbenchScreen.Pairing && next != WorkbenchScreen.Pairing) pairing.leave()
        if (next == WorkbenchScreen.Pairing) pairing.enter()
        if (next == WorkbenchScreen.Projects && project != null) {
            if (pending != 0) { message = "Finishing the current edit. Try returning to projects in a moment."; return }
            busy = true
            scope.launch {
                var instructionSeal = false
                try {
                    closeAssisted()
                    val closed = mutations.withLock {
                        if (!instructionEditor.sealIfClean()) {
                            message = "Save or discard the instruction draft before closing this project."
                            screen = WorkbenchScreen.Instructions; false
                        } else {
                            instructionSeal = true; closeCurrent(); true
                        }
                    }
                    if (closed) { refreshProjects(); screen = next }
                }
                catch (error: Exception) { report(error) }
                finally {
                    if (instructionSeal && !disposing) {
                        instructionEditor.detach(); instructionEditor.resume(); semanticEditor.resumeAfterReplacement(); instructionEditor.refresh(); instructionFocus.revisionChanged()
                    }
                    busy = false
                }
            }
        } else screen = next
    }
    fun returnFromSettings() { captureNavigationGeneration++; screen = if (project == null) WorkbenchScreen.Projects else WorkbenchScreen.Canvas }
    fun onBackground() {
        interruptAirInput()
        invalidateAssisted();scope.launch { try{closeAssisted()}catch(error:Exception){report(error)} }
        remoteCapability?.seal("background")
        captureNavigationGeneration++
        ai.background();instructionInteractionChanged()
        instructionFocus.sharing(false); instructionFocus.following(false)
        selections.background()
        if (erasers.state.value.busy) erasers.interaction.cancelOperation()
        cancelTool(); finishRemotePreview(sliderLink, sliderGesture, true)
        sliderGesture = null; sliderLink = null; sliderRevision = null; selectedStyleDraft = null; showBrush = false
        pairing.leave(); diagnostics.background(); hoverPoint = null; publish()
    }

    private suspend fun refreshProjects() {
        projects = repository.list().map { ProjectCard(it.path, it.info?.title ?: "Unavailable project",
            if (it.info == null) "The saved files are preserved. Tap to retry opening." else "${it.info.documentIds.size} document · saved on this phone",
            ProjectRepository.modified(it.modifiedMs)) }
    }
    fun newCanvas() = openRequest(knownSrgb = true) { repository.createBlank() }
    fun importDocument(uri:Uri,kind:EditableKind,password:String?=null) = openRequest(transfer=true) {
        documentOwner.ensureJobs()
        documentOwner.io.import(uri,kind,repository.receivePath(),repository.deviceId,"Imported document",password){transferLabel=it}
    }
    fun publishDocument(uri:Uri,copy:DocumentExport) {
        if(!documents.exportPublicationAvailable||busy||pending!=0){selections.discardUnused(uri);return}
        busy=true
        transferJob=scope.launch {
            try { documentOwner.io.publish(uri,copy){documents.checkedExport()===copy&&!disposing};message="Document copy saved." }
            catch(cancel:CancellationException){throw cancel}
            catch(error:Exception){report(error)}
            finally{busy=false;transferJob=null}
        }
    }
    fun importImage(uri: Uri) = openRequest(transfer = true) { repository.importImage(uri) { transferLabel = it } }
    /** The large-JPEG picker supplies a transient URI grant. The native URI
     * reader admits it before provider open; no Java-side original is spooled. */
    fun receiveGiantImage(uri: Uri, assumeUntaggedSrgb: Boolean) {
        if (!assumeUntaggedSrgb) { message = "Confirm the large-JPEG color assumption before choosing an original."; return }
        receivePickedImage(uri, {}, true)
    }
    /** Copy transient picker/share grants while this process owns them; no persistent grant is needed. */
    fun receiveImage(uri: Uri, cleanup: suspend (ImportOutcome) -> Unit = {}) = receivePickedImage(uri, cleanup, null)
    private fun receivePickedImage(uri: Uri, cleanup: suspend (ImportOutcome) -> Unit, giantSrgb: Boolean?) {
        if (disposing || incomingJob?.isActive == true) {
            message = "Finish the current image import before opening another image."
            scope.launch { cleanupTransfer { cleanup(ImportOutcome.Retained) }; refreshCaptures() }
            return
        }
        explicitImportCancel = false
        incomingJob = scope.launch {
            var importer: Job? = null
            var imported = false
            try {
                require(uri.scheme == "content") { "Choose an image from a content provider." }
                if (importReadiness.awaitReady() && !disposing) {
                    importer = openRequest(knownSrgb = giantSrgb == true, transfer = true) {
                        val handle = if (giantSrgb == null) repository.importImage(uri) { transferLabel = it }
                        else {
                            // A failed native URI-reader close can retain either
                            // of two input permits before a picked owner exists.
                            // Retry those exact readers once before new admission.
                            retryAndroidGiantUriRetirement()
                            val now = System.currentTimeMillis()
                            val id = core.newId(now.toULong())
                            val picked = openAndroidGiantOriginal(getApplication(), GiantUriOptions(
                                uri.toString(), getApplication<Application>().cacheDir.absolutePath,
                                2uL * 1024uL * 1024uL * 1024uL, 4uL * 1024uL * 1024uL * 1024uL, giantSrgb))
                            publishAndroidGiantPicked(picked,
                                GiantCacheBuildOptions(2uL * 1024uL * 1024uL * 1024uL, 120_000uL, giantSrgb),
                                { GiantProjectOptions(repository.checkedPath(id), id,
                                    core.newId(now.toULong()), core.newId(now.toULong()), repository.deviceId,
                                    "Large JPEG", now, 2uL * 1024uL * 1024uL * 1024uL) }) { transferLabel = it }
                        }
                        imported = true
                        handle
                    }
                    incomingProducer = importer
                    importer?.join()
                }
            } catch (cancel: CancellationException) { importer?.cancel(); throw cancel }
            catch (error: Exception) { report(error) }
            finally {
                // The camera/provider lease must outlive the actual native
                // producer, including cancellation-safe native settlement.
                withContext(NonCancellable) {
                    importer?.join()
                    cleanupTransfer { cleanup(if (imported) ImportOutcome.Imported else if (explicitImportCancel) ImportOutcome.Cancelled else ImportOutcome.Retained) }
                    if (incomingProducer === importer) incomingProducer = null
                    try { refreshCaptures() } catch (_: Exception) { message = "Retained camera captures could not be listed. They remain on this phone." }
                }
            }
        }
    }
    fun open(id: String) = openRequest { repository.open(id) }
    private fun openRequest(knownSrgb: Boolean = false, transfer: Boolean = false, receivingProject: Boolean = false,
        onOpened: ((WorkbenchProject, Long) -> Unit)? = null, load: suspend () -> WorkbenchProject): Job? {
        if(workflows.blocksInput()||!workflows.mayLeave()||imageFrames.state.value.blocksInput)return null
        if(!mayCloseAi())return null
        if (instructionEditor.hasUnsaved()) {
            message = "Save or discard the instruction draft before opening another project."; screen = WorkbenchScreen.Instructions; return null
        }
        if (disposing || busy || pending != 0 || instructionEditor.state.value.busy) return null
        busy = true; cancelTool()
        val job = scope.launch {
            var instructionSeal = false
            var completion: ProjectOpenCompletion<WorkbenchProject>? = null
            try {
                closeAssisted()
                val opened=mutations.withLock {
                    if(disposing)return@withLock false
                    if (!instructionEditor.sealIfClean()) {
                        message = "Save or discard the instruction draft before opening another project."
                        screen = WorkbenchScreen.Instructions; return@withLock false
                    }
                    instructionSeal = true
                    closeCurrent()
                    val next=load()
                    try { currentCoroutineContext().ensureActive() }
                    catch (error: Throwable) {
                        withContext(NonCancellable) { try { next.close() } catch (retirement: Throwable) {
                            if (retirement !== error) error.addSuppressed(retirement)
                        } }
                        throw error
                    }
                    if(disposing){withContext(NonCancellable){next.close()};false}else{project=next;aiRetiring=false;true}
                }
                if(!opened)return@launch
                val current = checkNotNull(project)
                val token = epoch
                info = current.info()
                val doc = info?.documentIds?.firstOrNull() ?: error("This project has no image document")
                val snapshot = current.document(doc)
                val drawing = preparedDrawing(snapshot.render.items)
                if (disposing || token != epoch || current !== project) return@launch
                document = snapshot; info = snapshot.render.revision; objects = drawing
                documentOwner.bind(current,snapshot)
                instructionEditor.refresh(); instructionFocus.revisionChanged(); semanticEditor.refresh()
                observeProject(current, token)
                colorAssumed = knownSrgb; colorAssumedSourceAsset=null; screen = WorkbenchScreen.Canvas; cameraFitted = false
                fit(); bindWorkflows();enterWorkflows();refreshOriginalFormatNotice()
                if(knownSrgb)colorAssumedSourceAsset=originalFormat?.sourceAssetId
                loadBackground()
                if (onOpened != null) completion = ProjectOpenCompletion(current, token)
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: SessionFailure) {
                if (receivingProject) message = projectReceiveFailureMessage(error.kind) else report(error)
            }
            catch (error: CoreFailure) {
                if (transfer && !receivingProject) message = rasterFailure(error, importing = true) else report(error)
            }
            catch (error: Exception) { report(error) }
            finally {
                if (instructionSeal && !disposing) {
                    instructionEditor.detach(); instructionEditor.resume(); semanticEditor.resumeAfterReplacement(); instructionEditor.refresh(); instructionFocus.revisionChanged()
                }
                busy = false; if (transfer) { transferLabel = null; transferJob = null }; publish()
            }
            currentCoroutineContext().ensureActive()
            if (!disposing && document != null && screen == WorkbenchScreen.Canvas && onOpened != null) {
                completion?.deliverIfCurrent(project, epoch, onOpened)
            }
        }
        if (transfer) transferJob = job.takeIf { it.isActive }
        return job
    }

    private var captureOpenWork: Job? = null
    private var failedCaptureOpen: ULong? = null
    private var captureNavigationGeneration = 0L
    private var navigationContactActive = false
    override fun navigationContact(active: Boolean) {
        if (navigationContactActive == active) return
        navigationContactActive = active
        if (active) cancelTool()
        captureNavigationGeneration++
        instructionInteractionChanged()
    }
    private fun receivedCaptureIdle(ownInstructions: Boolean = false, ownSemantics: Boolean = false): Boolean = !disposing && !imageFrames.state.value.blocksInput && !navigationContactActive && pending == 0 && gesture == null && strokes.isEmpty() &&
        !ai.state.value.open && !instructionEditor.hasUnsaved() && (ownInstructions || !instructionEditor.state.value.busy) && (ownSemantics || !semanticEditor.state.value.busy) &&
        !selections.state.value.active && !erasers.state.value.active &&
        instructionAdmission.allows(textDraft != null, showBrush, showContext, blockingMessage != null)
    internal fun canOpenReceivedCapture(): Boolean = !workflows.blocksInput()&&!imageFrames.state.value.blocksInput&&!busy && captureOpenWork == null && receivedCaptureIdle()
    internal fun autoOpenReceivedCapture(value: ReceivedCapture): Boolean =
        screen == WorkbenchScreen.Canvas && failedCaptureOpen != value.createdHostSeq && canOpenReceivedCapture()

    /** Prepare display bytes before changing any UI document. Both mutation and
     * refresh owners are held, so an old snapshot cannot replace this document. */
    internal fun openReceivedCapture(value: ReceivedCapture) {
        if (!canOpenReceivedCapture() || !value.originalVerified || value.createdHostSeq == 0uL || captureArrivals.state.value != value) return
        val current = project ?: return
        val token = epoch
        val navigation = captureNavigationGeneration
        val enteredScreen = screen
        busy = true
        val nextWork = scope.launch(start = CoroutineStart.LAZY) {
            try {
                mutations.withLock {
                    refreshMutex.withLock capture@{
                        var sealed = false
                        var staged: Bitmap? = null
                        try {
                            fun stillOwned() = !disposing && project === current && epoch == token &&
                                captureNavigationGeneration == navigation && screen == enteredScreen && captureArrivals.state.value == value
                            if (!stillOwned() || !receivedCaptureIdle() || !instructionEditor.sealIfClean()) return@capture
                            sealed = true
                            val latest = captureArrivalQuery(current, value.createdHostSeq - 1uL)
                            if (latest == null || !latest.originalVerified || latest.createdHostSeq != value.createdHostSeq ||
                                latest.sourceAssetId != value.sourceAssetId || latest.frameId != value.frameId ||
                                latest.captureSessionId != value.captureSessionId || latest.geometryRevision != value.geometryRevision ||
                                latest.binding.documentId != value.binding.documentId) {
                                failedCaptureOpen = value.createdHostSeq; captureArrivals.changed(current, token); return@capture
                            }
                            val next = current.document(value.binding.documentId)
                            val drawing = preparedDrawing(next.render.items)
                            staged = captureDisplayBitmap(current.background(next.documentId, memoryBudgetBytes = 96uL * 1024uL * 1024uL, assumeUntaggedSrgb = false))
                            val checked = current.info()
                            if (!stillOwned() || !receivedCaptureIdle(ownInstructions = true) || checked.hostSeq != next.render.revision.hostSeq ||
                                checked.stateHash != next.render.revision.stateHash) return@capture
                            imageFrames.detach();workflows.detach();rotation.detach();semanticEditor.detach(); instructionEditor.detach(); detachGiant(); selections.detach(); erasers.detach(); retireAi()
                            // Modal entry may run while the owners above settle.
                            if (!stillOwned() || !receivedCaptureIdle(ownInstructions = true, ownSemantics = true)) return@capture
                            cancelTool(); invalidateExport(); clearPeerObjects(); aiRetiring = false
                            refreshAdmission.invalidate();documentRefresh.invalidate()
                            document = next; info = next.render.revision; objects = drawing; selected = null
                            background = staged; staged = null; composedBackground=null; blockingMessage = null; needsColorConsent = false; colorAssumed = false; colorAssumedSourceAsset=null
                            originalFormat = null; originalFormatNotice = null
                            screen = WorkbenchScreen.Canvas; cameraFitted = false; fit();bindWorkflows();enterWorkflows();refreshOriginalFormatNotice()
                            captureArrivals.acknowledge(value); failedCaptureOpen = null
                            message = "Received lossless capture. Its exact original is saved in this project."
                            publish()
                        } finally {
                            staged?.recycle()
                            withContext(NonCancellable) {
                                if (sealed && !disposing) {
                                    if (project === current && epoch == token) aiRetiring = false
                                    instructionEditor.detach(); instructionEditor.resume(); semanticEditor.resumeAfterReplacement()
                                    instructionEditor.refresh(); instructionFocus.revisionChanged(); semanticEditor.refresh()
                                }
                            }
                        }
                    }
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) {
                failedCaptureOpen = value.createdHostSeq
                message = "The capture original is saved, but display preparation failed or exceeded this phone's limit. The current document is unchanged. Open capture retries."
            }
        }
        captureOpenWork = nextWork
        nextWork.invokeOnCompletion {
            // Includes cancellation before the coroutine body starts.
            if (captureOpenWork === nextWork) { captureOpenWork = null; busy = false; publish() }
        }
        nextWork.start()
    }
    private suspend fun closeCurrent() {
        interruptAirInput(clearView = true, clearBarrel = true)
        withContext(NonCancellable) {
        closeAssisted();imageFrames.detach();workflows.detach();rotation.detach()
        documentOwner.detach()
        if(!precisionClosed)precision.reset()
        scene=null;composedBackground=null
        exportReady = null
        invalidateExport()
        detachGiant()
        erasers.detach(); selections.detach()
        instructionFocus.detach()
        semanticEditor.detach()
        instructionEditor.detach()
        retireAi()
        captureArrivals.detach(); failedCaptureOpen = null
        epoch++; refreshAdmission.reset();documentRefresh.invalidate()
        changesJob?.cancelAndJoin(); changesJob = null
        refreshQueue?.close(); refreshQueue = null
        refreshJob?.cancelAndJoin(); refreshJob = null
        try { detachLink() }
        finally {
            previewJob?.cancel(); cancelNative(previewHandle); previewHandle = null
            val previous = project; project = null
            try { previous?.close() }
            finally {
                document = null; info = null
                background = null; composedBackground = null; colorAssumed=false; colorAssumedSourceAsset=null; objects = emptyList(); selected = null; transientShape = null
                transformed = emptyMap(); erased = emptySet(); scene = null; blockingMessage = null; needsColorConsent = false; originalFormatNotice = null; originalFormat = null
            }
        }
        // Published render snapshots may still reference the old bitmap. Allow
        // normal reachability to release it instead of recycling under a draw.
        }
    }

    private fun observeProject(current: WorkbenchProject, token: Long) {
        captureArrivals.bind(current, token, checkNotNull(info).hostSeq)
        val queue = Channel<Unit>(Channel.CONFLATED)
        refreshQueue = queue
        refreshJob = scope.launch {
            for (ignored in queue) {
                if (token != epoch || current !== project || disposing) break
                try { refresh(current, current.info(), token) }
                catch (cancel: CancellationException) { throw cancel }
                catch (error: Exception) { report(error) }
            }
        }
        changesJob = scope.launch {
            try {
                current.changes.collect { change ->
                    if (change.kind in setOf(ChangeKind.Committed, ChangeKind.Opened) && token == epoch && current === project && change.project.projectId == info?.projectId &&
                        refreshAdmission.observe(change.sequence)) {
                        captureArrivals.changed(current, token)
                        giantOwner.observe(change.project)
                        invalidateAssisted()
                        if(composedBackground!=null && document?.render?.revision?.let { old ->
                            old.projectId==change.project.projectId && (old.hostSeq!=change.project.hostSeq || old.stateHash!=change.project.stateHash) }==true){
                            composedBackground=null;background=null
                        }
                        imageFrames.observe(change.sequence)
                        workflows.observe(change.project,change.sequence)
                        erasers.observe(change); selections.observe(change)
                        clearPeerObjects(); publish()
                        invalidateExport(); queue.trySend(Unit)
                    }
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { if (token == epoch && !disposing) report(error) }
        }
    }

    fun connectCurrent() {
        if (busy || disposing || pending != 0) return
        val current = project ?: run { message = "Open a project or receive the computer's project first."; return }
        val peer = pairing.selectedPeer ?: run { message = "Choose a paired computer first."; return }
        val endpoints = try { pairing.endpoint() } catch (error: Exception) { report(error); return }
        connectProject(current, epoch, peer, endpoints)
    }

    private fun connectProject(current: WorkbenchProject, token: Long, peer: String, endpoints: List<SessionEndpoint>) {
        if (busy || disposing || pending != 0 || current !== project || token != epoch) return
        busy = true
        connectionJob = scope.launch {
            try {
                mutations.withLock {
                    if (disposing || project !== current || token != epoch) return@withLock
                    detachLink()
                    val sessions = pairing.service()
                    val role = sessions.projectRole(current)
                    if (!role.isHost && role.hostDeviceId != peer) {
                        message = "This project belongs to a different host. Choose its paired computer."; return@withLock
                    }
                    val created = if (role.isHost) sessions.host(current, peer, endpoints) else sessions.connect(current, peer, endpoints)
                    if (disposing || project !== current || token != epoch) { withContext(NonCancellable) { created.close() }; return@withLock }
                    link = created; observeLink(created, current, token, ++linkEpoch)
                    instructionFocus.attach(created)
                    message = if (role.isHost) "Listening for the paired computer on the selected phone address." else "Connecting to the paired computer."
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: SessionFailure) { message = sessionMessage(error.kind) }
            catch (error: Exception) { report(error) }
            finally { busy = false }
        }
    }

    fun receiveProject() {
        val peer = pairing.selectedPeer ?: run { message = "Choose a paired computer first."; return }
        val endpoints = try { pairing.endpoint() } catch (error: Exception) { report(error); return }
        openRequest(transfer = true, receivingProject = true,
            onOpened = { current, token -> connectProject(current, token, peer, endpoints) }) {
            transferLabel = "Receiving and verifying the computer's project…"
            pairing.service().receiveProject(repository.receivePath(), peer, endpoints)
        }
    }

    private fun observeLink(created: ProjectLink, current: WorkbenchProject, token: Long, generation: Long, reattachAgent:Boolean=true) {
        clearPeerObjects()
        connection = created.status(); lastViewport = null
        if(reattachAgent)agentCapture.attach(created)
        linkChanges = scope.launch {
            try {
                created.changes.collect { status ->
                    if (token != epoch || generation != linkEpoch || project !== current || link !== created) return@collect
                    if (connection?.let { status.sequence <= it.sequence } == true) return@collect
                    val before = connection
                    connection = status
                    agentCapture.connectionChanged(status)
                    if (before?.status != status.status || before?.carrier != status.carrier ||
                        status.status == SyncStatus.Reconnecting || status.status == SyncStatus.Offline) clearPeerObjects()
                    if (before?.status != status.status) lastViewport = null
                    if (before?.stateHash != status.stateHash || before?.hostSeq != status.hostSeq) refreshQueue?.trySend(Unit)
                    if (followPeer) matchPeerCamera()
                    publish()
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: SessionFailure) { if (link === created) message = sessionMessage(error.kind) }
        }
        peerWorker = scope.launch {
            var lastSequence: ULong? = null
            var lastAdmission = peerAdmission
            try {
                while (isActive && link === created) {
                    try {
                    val requested = document
                    val admission = peerAdmission
                    val ticket = refreshAdmission.capture()
                    if (lastAdmission != admission) { lastSequence = null; lastAdmission = admission }
                    if (requested != null && connection?.status in setOf(SyncStatus.Syncing, SyncStatus.Synced)) {
                        val preview = created.peerPreviews(requested.documentId)
                        if (lastSequence != preview.sequence) {
                            val drawing = preparedDrawing(preview.items)
                            // A returned query belongs to both a visible revision and
                            // a connected status generation. Neither later OPS nor an
                            // offline/reconnect transition may resurrect its overlay.
                            if (token == epoch && generation == linkEpoch && project === current && link === created &&
                                document === requested && admission == peerAdmission && refreshAdmission.accepts(ticket) &&
                                connection?.status in setOf(SyncStatus.Syncing, SyncStatus.Synced)) {
                                lastSequence = preview.sequence; peerObjects = drawing; publish()
                            }
                        }
                    } else { lastSequence = null }
                    } catch (error: SessionFailure) {
                        if (error.kind == SessionFailureKind.Closed) break
                        if (error.kind !in setOf(SessionFailureKind.Backpressure, SessionFailureKind.Transport, SessionFailureKind.Timeout)) throw error
                        lastSequence = null; clearPeerObjects(); publish()
                    }
                    delay(16)
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { if (link === created) { clearPeerObjects(); publish(); report(error) } }
        }
        publish()
    }

    private fun clearPeerObjects() { peerAdmission++; peerObjects = emptyList() }

    private suspend fun detachLink() {
        interruptAirInput()
        withContext(NonCancellable) {
        agentCapture.detach()
        instructionFocus.detach()
        linkEpoch++; followPeer = false
        linkChanges?.cancelAndJoin(); linkChanges = null
        peerWorker?.cancelAndJoin(); peerWorker = null
        val previous = link; remoteCapability?.seal("connection_retired")
        connection = null; clearPeerObjects(); lastViewport = null
        if (previous != null) withContext(NonCancellable) { previous.close() }
        // Pending close retains the actual Link and UI capability for retry.
        if(link===previous){link=null;remoteCapability=null;remoteOpen=false}
        }
    }
    fun disconnect() {
        if (busy) return
        cancelTool(); scope.launch { try { detachLink(); publish() } catch (error: Exception) { report(error) } }
    }
    fun revoke(peer: String) { pairing.revoke(peer) { cancelTool(); detachLink(); publish() } }
    fun matchPeer() { if(workflows.blocksInput()||imageFrames.state.value.blocksInput)return;followPeer = false; matchPeerCamera(); publish() }
    fun followPeer(enabled: Boolean) { if(workflows.blocksInput()||imageFrames.state.value.blocksInput)return;followPeer = enabled; if (enabled) matchPeerCamera(); publish() }
    private fun matchPeerCamera() {
        val view = connection?.peerViewport?.takeIf { it.documentId == document?.documentId && it.corners.size == 4 } ?: return
        val first = view.corners[0]; val next = view.corners[1]
        val width = hypot(next.x - first.x, next.y - first.y)
        if (!width.isFinite() || width <= 0 || view.corners.any { !it.x.isFinite() || !it.y.isFinite() }) return
        cancelTool(); invalidateViewCheck()
        camera = camera.copy(center = Point(view.corners.sumOf { it.x } / 4, view.corners.sumOf { it.y } / 4),
            scale = (camera.viewportWidth / width).coerceIn(.001, 128.0), rotation = -atan2(next.y - first.y, next.x - first.x))
        unsnappedRotation = camera.rotation
    }
    fun connectionLabel(): String = connection?.let {
        when (it.status) {
            SyncStatus.Synced -> "SYNCED"
            SyncStatus.Syncing -> "SYNCING (${it.pending} pending)"
            SyncStatus.Reconnecting -> "RECONNECTING"
            SyncStatus.Offline -> "OFFLINE (${it.pending} pending)"
        } + if (it.blocked > 0u) " · ${it.blocked} blocked edits" else ""
    } ?: "OFFLINE · Saved on this phone"
    private fun sendObject(target: ProjectLink?, preview: ObjectPreview) {
        try { target?.streamObject(preview) } catch (_: SessionFailure) { /* Latest-only preview; commit stays durable. */ }
    }
    private fun finishRemotePreview(target: ProjectLink?, id: String?, cancel: Boolean) {
        if (target == null || id == null) return
        scope.launch { try { target.finishPreview(id, cancel) } catch (_: SessionFailure) { /* A disconnected epoch retires all previews. */ } }
    }

    private suspend fun refreshOriginalFormatNotice() {
        if(documents.state.value.descriptor!=null){originalFormatNotice=null;originalFormat=null;return}
        val owner = project ?: return
        val formats = owner as? WorkbenchOriginalFormats ?: return
        val snapshot = document ?: return
        val revision = snapshot.render.revision
        val binding = WorkflowBinding(revision.projectId, snapshot.documentId, revision.hostSeq, revision.stateHash)
        val token = epoch
        val ticket = refreshAdmission.capture()
        fun current(): Boolean = !disposing && project === owner && epoch == token && refreshAdmission.accepts(ticket) &&
            document?.let { it.documentId == binding.documentId && originalFormatRevisionMatches(it.render.revision, binding) } == true
        if (!current()) return
        originalFormatNotice = null; originalFormat = null
        try {
            val receipt = formats.originalFormatInfo(binding)
            currentCoroutineContext().ensureActive()
            validateOriginalFormatInfo(receipt, binding)
            if (current()) { originalFormat = receipt; originalFormatNotice = originalFormatNotice(receipt) }
        } catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) {
            currentCoroutineContext().ensureActive()
            if (current()) originalFormatNotice = "Original-image page information could not be checked. Reopen the project to retry."
        }
    }
    private suspend fun loadBackground() {
        val current = project ?: return
        val doc = document ?: return
        val token = epoch
        val ticket = refreshAdmission.capture()
        val revision = doc.render.revision
        val binding = WorkflowBinding(revision.projectId,doc.documentId,revision.hostSeq,revision.stateHash)
        fun stillCurrent(): Boolean = !disposing && project === current && token == epoch && refreshAdmission.accepts(ticket) &&
            document?.let { it.documentId == doc.documentId && it.page == doc.page &&
                WorkflowBinding(it.render.revision.projectId,it.documentId,it.render.revision.hostSeq,it.render.revision.stateHash) == binding } == true
        if(documents.state.value.descriptor!=null){detachGiant();background=null;composedBackground=null;blockingMessage=null;needsColorConsent=false;publish();return}
        if(giantDisplayRequired(doc)){loadGiantBackground(current,doc);return}
        if(giantMode||giantOwner.retaining)detachGiant()
        val composed = doc.hasToneAdjustments
        val format = originalFormat?.takeIf { it.binding == binding }
        if(composed){
            background=null;composedBackground=null
            if(format==null || format.rawPreview){
                needsColorConsent=false
                blockingMessage=if(format?.rawPreview==true) "Saved tone composition is unavailable for an embedded RAW preview. The sensor original remains unchanged."
                    else "The saved tone composition needs verified source information. Reopen the project to retry."
                publish();return
            }
            if(format.untagged && !approvedAssumption(doc)){
                needsColorConsent=true
                blockingMessage="Tone display requires explicit sRGB consent for this untagged original. The original remains unchanged."
                publish();return
            }
        }
        try {
            val assumed = approvedAssumption(doc)
            val image = if(composed){
                val value=imageFrameWorkflows(current).editingBackground(binding,assumed)
                if(value.binding!=binding || value.width!=doc.width || value.height!=doc.height ||
                    value.sourceBitDepth.toUInt()!=doc.bitDepth || value.sourceAssetId.length!=64 ||
                    value.sourceAssetId.any { it !in "0123456789abcdef" } ||
                    value.rgba.size.toULong()!=doc.width.toULong()*doc.height.toULong()*4uL ||
                    value.sourceAssetId!=format?.sourceAssetId) throw WorkflowFailure(WorkflowFailureKind.Stale)
                BackgroundImage(value.width,value.height,value.rgba,value.sourceAssetId,value.sourceBitDepth,value.iccProfile)
            }else current.background(doc.documentId, assumeUntaggedSrgb = assumed)
            currentCoroutineContext().ensureActive()
            if(!stillCurrent())return
            val bitmap = withContext(Dispatchers.Default) {
                require(image.width in 1u..50_000u && image.height in 1u..50_000u)
                val width = image.width.toInt(); val height = image.height.toInt()
                require(width.toLong() * height * 4 == image.rgba.size.toLong())
                val out = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
                try {
                    val row = IntArray(width)
                    for (y in 0 until height) {
                        ensureActive()
                        for (x in row.indices) {
                            val i = (y * width + x) * 4
                            row[x] = ((image.rgba[i + 3].toInt() and 255) shl 24) or ((image.rgba[i].toInt() and 255) shl 16) or
                                ((image.rgba[i + 1].toInt() and 255) shl 8) or (image.rgba[i + 2].toInt() and 255)
                        }
                        out.setPixels(row, 0, width, 0, y, width, 1)
                    }
                    out
                } catch (error: Exception) { out.recycle(); throw error }
            }
            var adopted=false
            try {
                currentCoroutineContext().ensureActive()
                if(!stillCurrent())return
                background = bitmap
                composedBackground = if(composed) ComposedBackgroundOwner(current,token,doc.documentId,doc.page,binding) else null
                blockingMessage = null; needsColorConsent = false
                adopted=true
            } finally { if(!adopted) bitmap.recycle() }
        } catch (cancel: CancellationException) { throw cancel }
        catch (error: CoreFailure) {
            if(!stillCurrent())return
            needsColorConsent = error.kind == CoreFailureKind.Raster && !approvedAssumption(doc) && format?.untagged==true
            blockingMessage = if (error.kind == CoreFailureKind.Backpressure)
                "This image exceeds the current full-resolution display memory budget. Its original is saved unchanged. Bounded tiled display is still required before all images up to 50 megapixels can be edited."
            else if (error.kind == CoreFailureKind.ImageCodecUnavailable)
                "A compatible image decoder is unavailable on this device. The original is preserved."
            else if (needsColorConsent)
                "The image could not be converted for display. If it has no embedded color profile, you can explicitly treat its colors as sRGB. The original will remain unchanged."
            else "The image could not be displayed safely. Its original remains saved; no smaller replacement was created."
        } catch(error: Exception){
            currentCoroutineContext().ensureActive()
            if(!stillCurrent())return
            blockingMessage=if(composed) "The saved tone composition could not be displayed. Retry or reopen before editing; saved data is intact."
                else "The image could not be displayed safely. Its original remains saved."
        }
        publish()
    }
    fun assumeSrgb() {
        if (busy || !needsColorConsent) return
        val source=originalFormat?.sourceAssetId ?: run { message="The original source could not be checked. Reopen the project before approving its colors.";return }
        colorAssumed = true; colorAssumedSourceAsset=source; busy = true; invalidateExport()
        scope.launch { try { loadBackground() } catch (error: Exception) { report(error) } finally { busy = false } }
    }

    fun viewport(width: Int, height: Int, displayDensity: Float) {
        if (width <= 0 || height <= 0) return
        if (camera.viewportWidth != width.toDouble() || camera.viewportHeight != height.toDouble()) invalidateViewCheck()
        cancelTool(); density = displayDensity
        camera = camera.copy(viewportWidth = width.toDouble(), viewportHeight = height.toDouble())
        if (!cameraFitted) fit() else publish()
    }
    fun fit() {
        val doc = document ?: return
        invalidateViewCheck(); followPeer = false
        cancelTool()
        camera = camera.copy(center = Point(doc.width.toDouble() / 2, doc.height.toDouble() / 2),
            scale = min(camera.viewportWidth / doc.width.toDouble(), camera.viewportHeight / doc.height.toDouble()).coerceIn(.001, 128.0) * .92,
            rotation = 0.0)
        cameraFitted = camera.viewportWidth > 1 && camera.viewportHeight > 1
        unsnappedRotation = 0.0
        publish()
    }
    private fun renderScene(value: CanvasScene): CanvasScene {
        val composed = if(value.giantMode)value.giantFrame?.content==GiantTileContent.Composed
            else composedCurrent(value.document)
        val baked=bakedCanonicalIds(value.document,composed)
        return if(baked.isEmpty())value else value.copy(hidden=value.hidden+baked,
            provisional=value.provisional.filterNot { it.item.objectId in baked })
    }
    private fun publish() {
        if(synchronizingPrecision||updatingGiant)return
        if (airInitialized && airObservedPage != documents.state.value.page) {
            airObservedPage = documents.state.value.page; interruptAirInput(clearView = true, clearBarrel = true)
        }
        if(giantMode&&(scene?.camera!=camera||scene?.document!==document||scene?.precision!=precision.frame())){giantDraw.set(null);giantRenderReady=false}
        ai.changed()
        val doc = document ?: return
        project?.let { selections.bind(it, doc, epoch, refreshAdmission.capture().sequence, camera); erasers.bind(it, doc, epoch, refreshAdmission.capture().sequence, camera) }
        if (camera.viewportWidth <= 1 || camera.viewportHeight <= 1) return
        if(precision.baseCamera()!=camera){
            synchronizingPrecision=true
            try{cancelTool();precision.synchronize(camera)}finally{synchronizingPrecision=false}
        }
        aiResults.request()
        val resultFrame=aiFrame()
        val savedIds = objects.mapTo(HashSet()) { it.item.objectId }
        val admittedPeers=peerObjects.filter{aiAdmitResultPreview(doc,it.item)}
        val peerById = admittedPeers.associateBy { it.item.objectId }
        // Existing objects keep their canonical stacking position. Local
        // transforms and erasures below still override the remote replacement.
        val visibleObjects = objects.map { peerById[it.item.objectId] ?: it }
        val localProvisional = (strokes.mapNotNull { it.drawing } + pendingPreviews.values).filter { it.item.objectId !in savedIds } + listOfNotNull(transientShape)
        val localIds = localProvisional.mapTo(HashSet()) { it.item.objectId }
        val provisional = localProvisional + admittedPeers.filter { it.item.objectId !in savedIds && it.item.objectId !in localIds }
        val allTransforms = pendingTransforms.values.fold(emptyMap<String, Transform>()) { a, b -> a + b } + transformed + rotation.state.value.preview
        val allErased = pendingErases.values.fold(emptySet<String>()) { a, b -> a + b } + erased
        var giantUpdateFailed=false
        giantOwner.current?.let{view->
            updatingGiant=true
            try{view.update(core,camera,precision.frame());giantIssue=view.status.message}
            catch(_:Exception){giantUpdateFailed=true;giantDraw.set(null);giantIssue="This tiled viewport exceeds its display allowance. Reduce the viewport or retry display."}
            finally{updatingGiant=false}
        }
        val giantFrame=if(giantUpdateFailed)null else giantFrame(doc)
        if(giantDraw.get()!==giantFrame)giantRenderReady=false
        giantDraw.set(giantFrame)
        val displayBackground=background.takeIf { !doc.hasToneAdjustments || composedCurrent(doc) }
        val next = CanvasScene(++sceneRevision, doc, displayBackground, camera, core.cameraMatrix(camera), visibleObjects,
            provisional, allErased, allTransforms, selected, hoverPoint, preferences.brush(tool).width, density, preferences.darkTheme,
            connection?.peerViewport?.takeIf { it.documentId == doc.documentId }?.corners.orEmpty(),
            aiResults=resultFrame?.images.orEmpty(),resultDisplayReady=aiResultReady(),documentFrame=documentOwner.matchingFrame(doc,epoch),precision=precision.frame(),documentDraft=documentPress,giantMode=giantMode,giantFrame=giantFrame,rotationHandle=rotationHandle())
        scene = next
        canvasSink?.invoke(renderScene(next))
        workflows.remember(camera)
        documents.viewport(camera)
        val viewport = PeerViewport(doc.documentId, core.mapPoints(camera, true, listOf(Point(0.0, 0.0),
            Point(camera.viewportWidth, 0.0), Point(camera.viewportWidth, camera.viewportHeight), Point(0.0, camera.viewportHeight))))
        if (viewport != lastViewport) {
            try { link?.sendViewport(viewport.documentId, viewport.corners); lastViewport = viewport }
            catch (_: SessionFailure) { /* Volatile state is retried on the next view change/reconnect. */ }
        }
    }

    private fun toDocument(sample: PenSample, camera: Camera) = core.mapPoints(camera, true, listOf(Point(sample.x, sample.y))).single()
    private fun selectedLayer(): LayerInfo = document?.let{documentControlDrawingLayer(it,workflows.controls.state.value.drawingLayerId)}
        ?: error("Choose a visible unlocked annotation layer before drawing")

    private class StrokeRun(val epoch: Long, val camera: Camera, val start: PenSample, val origin: Point, val capabilities: StylusCapabilities,
                            val options: StrokeOptions, val layerId: String, val opacity: Double, val blend: String, val link: ProjectLink?) {
        val queue = Channel<List<PenSample>>(64)
        val canceled = AtomicBoolean(false)
        var native: WorkbenchStroke? = null
        var drawing: DrawObject? = null
        var prediction: PenSample? = null
        var real = Path()
        var polygons = 0uL
        var samples = 0
        var sequence = 1uL
        var lastTime = start.timeMs
        var finished = false
        var committed = false
    }
    private data class ToolGesture(val tool: EditorTool, val camera: Camera, val first: Point, val startScreen: Point,
                                   val layer: LayerInfo, val brush: BrushPreference, val stroke: StrokeRun? = null,
                                   val item: RenderItem? = null, val resizeAnchor: Point? = null,
                                   val previewId: String? = null, val link: ProjectLink? = null, var previewSequence: UInt = 0u,
                                   val revision: ProjectInfo? = null, val newObjectId: String? = null,
                                   val createdAtMs: Long = System.currentTimeMillis(), var semanticEnd: Point = first, val semanticModifier: Boolean = false,val physical:Transform=Transform())

    internal fun precisionInputAllowed():Boolean = !disposing&&!rotationUnavailable()&&!workflows.blocksInput()&&!imageFrames.state.value.blocksInput&&screen==WorkbenchScreen.Canvas&&!busy&&strokes.size<3&&
        !instructionEditor.state.value.busy&&!semanticEditor.state.value.busy&&!documents.state.value.busy&&
        !ai.state.value.open&&aiResultReady()&&blockingMessage==null&&textDraft==null&&!showBrush&&!showContext&&instructionAdmission.allows(false,false,false,false)&&
        document?.let{(!it.hasToneAdjustments || composedCurrent(it) || (giantMode && giantInputReady(it))) &&
            (background!=null||documentOwner.matchingFrame(it,epoch)!=null||giantInputReady(it))}==true
    override fun beginTool(rawSample: PenSample, capabilities: StylusCapabilities) {
        interruptAirInput()
        if(rotationUnavailable()||workflows.blocksInput()||imageFrames.state.value.blocksInput||ai.state.value.open||!aiResultReady())return
        val retainPolygon = !selectionPointer && !eraserPointer && gesture == null && documentPress == null &&
            precisionInputAllowed() && erasers.tools.route(rawSample.tool == PointerTool.Eraser) == EraserInputRoute.Selection &&
            selections.interaction.canResumePolygonContact(camera)
        clearToolContact(retainPolygon); previewJob?.cancel(); cancelNative(previewHandle); previewHandle = null
        if (busy || instructionEditor.state.value.busy || semanticEditor.state.value.busy || blockingMessage != null || (background == null && document?.let{documentOwner.matchingFrame(it,epoch)}==null && document?.let{giantInputReady(it)}!=true) || documents.state.value.busy || strokes.size >= 3) {
            if (strokes.size >= 3) message = "The save queue is full. Wait for the current strokes to finish."
            return
        }
        val current = project ?: return
        if(!precisionInputAllowed())return
        val route=erasers.tools.route(rawSample.tool==PointerTool.Eraser)
        if(route==EraserInputRoute.Blocked)return
        val raw=Point(rawSample.x,rawSample.y)
        precision.down(raw)
        val physical=precision.inputMatrix()
        val sample=precision.routePen(listOf(rawSample)).single()
        if(assistedCollecting && rawSample.tool==PointerTool.Pen){
            val actions=assistedActions
            val source=actions?.source
            if(actions!=null && source!=null && actions.state.value.canPrompt && assistedCurrent(current,source)){
                assistedTap=AssistedTap(current,source,camera,sample,assistedNegative)
            } else {assistedCollecting=false;message="The assisted source changed. Reopen assisted selection before drawing."}
            return
        }
        val documentMode=documents.state.value
        if(documentMode.textTool||documentMode.regionTool){
            val point=toDocument(sample,camera)
            documentPress=point to point;documentPressPage=documentMode.page;documentPressRevision=info;documentPressEpoch=epoch;documentPressRegion=documentMode.regionTool
            publish();return
        }
        when (route) {
            EraserInputRoute.Selection -> { selectionPointer = selections.interaction.begin(Point(sample.x, sample.y), camera);if(!selectionPointer)precision.end();return }
            EraserInputRoute.Eraser -> { eraserPointer = erasers.interaction.begin(Point(sample.x, sample.y), camera, sample.tool == PointerTool.Eraser,hypot(physical.a,physical.b));if(!eraserPointer)precision.end();return }
            EraserInputRoute.Blocked -> return
            EraserInputRoute.Editor -> Unit
        }
        this.capabilities = capabilities; hoverPoint = null
        if(tool==EditorTool.Select&&route==EraserInputRoute.Editor&&beginRotation(toDocument(sample,camera),raw,physical))return
        val activeTool = tool
        if (activeTool == EditorTool.Eraser) {precision.end();return} // all erasers use the guarded route above
        try {
            val layer = selectedLayer(); val brush = preferences.brush(activeTool); val first = toDocument(sample, camera)
            var run: StrokeRun? = null
            var item: RenderItem? = null
            var anchor: Point? = null
            if (activeTool in listOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker)) {
                val now = System.currentTimeMillis(); val gestureId = core.newId(now.toULong())
                val family = when (activeTool) { EditorTool.Highlighter -> "highlighter"; EditorTool.Marker -> "marker"; else -> "pen" }
                val explicitLayer=workflows.controls.state.value.drawingLayerId
                if(family=="highlighter"&&explicitLayer!=null&&layer.blend!="multiply"){
                    message="Choose a multiply annotation layer for highlighter marks.";precision.cancel();return
                }
                val highlighter = if(family!="highlighter")null else if(explicitLayer!=null)layer
                    else document?.layers?.filter { it.visible && !it.locked && it.blend == "multiply" }?.minByOrNull { it.id }
                val target = if (family == "highlighter") highlighter?.id ?: gestureId else layer.id
                val options = StrokeOptions(gestureId, core.newId(now.toULong()), core.newId(now.toULong()), checkNotNull(document).documentId,
                    layer.id, repository.deviceId, repository.lamport(3), now, family, brush.width, EditorDrawing.rgba(brush.colorArgb),
                    brush.stabilization, brush.pressure.values())
                run = StrokeRun(epoch, camera, sample, first, capabilities, options, target, highlighter?.opacity ?: layer.opacity,
                    if (family == "highlighter") "multiply" else "normal", link)
                invalidateExport(); strokes += run; pending++; startStroke(current, run)
            } else if (activeTool == EditorTool.Select) {
                val snapshot = scene?.let{precisionHitScene(it,physical)}
                val previous = objects.firstOrNull { it.item.objectId == selected }
                if (snapshot != null && previous != null) {
                    val near=precisionResizeHandle(snapshot,previous.item,raw,24*density)
                    if(near!=null){
                        item = previous.item
                        val r = item.bounds
                        val corners = listOf(Point(r.x, r.y), Point(r.x + r.width, r.y), Point(r.x + r.width, r.y + r.height), Point(r.x, r.y + r.height))
                        anchor = corners[(near + 2) % 4]
                    }
                }
                if (item == null) item = snapshot?.let { EditorDrawing.hit(it, raw, 8 * density)?.item }
                selected = item?.objectId
                instructionFocus.localSelection(selected)
                if (item?.shape is Shape.Marker) item?.objectId?.let(instructionEditor::focusObject)
            }
            if (activeTool == EditorTool.Select && item?.shape is Shape.Guide) {
                selected = item.objectId
                message = "Use pixel selection tools to edit coverage; guides keep their source coordinates."
                publish(); return
            }
            val newShape = activeTool in listOf(EditorTool.Line, EditorTool.Arrow, EditorTool.Rectangle, EditorTool.Ellipse)
            gesture = ToolGesture(activeTool, camera, first, Point(sample.x, sample.y), layer, brush, run, item, anchor,
                if (item != null || newShape) newId() else null, link, revision = info, newObjectId = if (newShape || activeTool == EditorTool.Callout) newId() else null, semanticModifier = sample.buttons and android.view.MotionEvent.BUTTON_STYLUS_SECONDARY != 0,physical=physical)
            eraserLast = if (activeTool == EditorTool.Eraser) Point(sample.x, sample.y) else null
            if (activeTool == EditorTool.Text) {
                val hit = scene?.let { EditorDrawing.hit(precisionHitScene(it,physical),raw,8*density)?.item }
                val text = hit?.shape as? Shape.Text
                textDraft = if (text != null) TextDraft(hit.objectId, text.anchor, text.text, text.font, text.size) else TextDraft(null, first)
            }
            extendMappedTool(listOf(sample)); publish()
        } catch (error: Exception) { cancelTool(); report(error) }
    }

    override fun extendTool(samples: List<PenSample>) {
        samples.lastOrNull()?.let{precision.move(Point(it.x,it.y))}
        if(assistedTap!=null)return
        val mapped=precision.routePen(samples)
        rotationGesture?.let{token->
            if(precision.inputMatrix()!=rotationPhysical){rotationGesture=null;rotation.cancel();publish();return}
            mapped.lastOrNull()?.let{sample->val point=toDocument(sample,token.frame.camera);rotationDisplay=rotationDisplay?.copy(point=point);rotation.move(token,point,rotationSnap15)}
            publish();return
        }
        if(documentPress!=null){val end=mapped.lastOrNull()?.let{toDocument(it,camera)};if(end!=null)documentPress=documentPress!!.first to end;publish();return}
        extendMappedTool(mapped)
    }
    private fun extendMappedTool(samples: List<PenSample>) {
        if (eraserPointer) { samples.forEach { erasers.interaction.append(Point(it.x, it.y)) }; return }
        if (selectionPointer) { samples.forEach { selections.interaction.append(Point(it.x, it.y)) }; return }
        val active = gesture ?: return
        if (samples.isEmpty()) return
        try {
            val run = active.stroke
            if (run != null) {
                if (!run.queue.trySend(samples.toList()).isSuccess) { cancelTool(); message = "Ink input exceeded its bounded queue. The unfinished stroke was canceled; saved marks are unchanged." }
                return
            }
            val end = toDocument(samples.last(), active.camera)
            when (active.tool) {
                EditorTool.Callout -> {
                    active.semanticEnd = end
                    transientShape = semanticDropBox(active.first, end, active.physical)?.let { previewShape(Shape.Rectangle(it), active) }
                }
                EditorTool.Line, EditorTool.Arrow, EditorTool.Rectangle, EditorTool.Ellipse -> {
                    val shape = shape(active.tool, active.first, end)
                    if (shape != null) {
                        transientShape = previewShape(shape, active)
                        val item = checkNotNull(transientShape).item
                        try { active.link?.streamNewObject(NewObjectPreview(checkNotNull(active.previewId), checkNotNull(document).documentId,
                            item.objectId, item.layerId, ++active.previewSequence, active.createdAtMs, item.shape, item.transform, item.style)) }
                        catch (_: SessionFailure) { /* The bounded native preview may decline; final edit remains available. */ }
                    }
                }
                EditorTool.Eraser -> {
                    val snapshot = scene ?: return
                    for (sample in samples) {
                        val next = Point(sample.x, sample.y)
                        erased = erased + EditorDrawing.eraseHits(snapshot.copy(hidden = erased), eraserLast ?: next, next,
                            max(4 * density, (active.brush.width * camera.scale / 2).toFloat()))
                        // Numbered markers own linked instructions and numbering.
                        // The explicit marker delete workflow must retire both.
                        val markers = objects.filter { it.item.shape is Shape.Marker }.map { it.item.objectId }.toSet()
                        if (erased.any { it in markers }) message = "Use Delete marker in Instructions to remove a numbered marker and its linked instruction."
                        erased = erased - markers
                        eraserLast = next
                    }
                    require(erased.size <= 512) { "Erase at most 512 objects in one gesture; this unfinished erase was canceled." }
                }
                EditorTool.Select -> active.item?.let { item ->
                    val outer = if (active.resizeAnchor == null) Transform(e = end.x - active.first.x, f = end.y - active.first.y) else {
                        val anchor = active.resizeAnchor
                        val dx = active.first.x - anchor.x; val dy = active.first.y - anchor.y
                        val sx = if (abs(dx) < .001) 1.0 else ((end.x - anchor.x) / dx).coerceIn(.05, 20.0)
                        val sy = if (abs(dy) < .001) 1.0 else ((end.y - anchor.y) / dy).coerceIn(.05, 20.0)
                        Transform(a = sx, d = sy, e = anchor.x * (1 - sx), f = anchor.y * (1 - sy))
                    }
                    transformed = mapOf(item.objectId to EditorDrawing.compose(outer, item.transform))
                    active.previewId?.let { id -> sendObject(active.link, ObjectPreview(id, checkNotNull(document).documentId,
                        item.objectId, ++active.previewSequence, PreviewKind.HandleDrag, checkNotNull(transformed[item.objectId]), item.style)) }
                }
                else -> Unit
            }
            publish()
        } catch (error: Exception) { cancelTool(); report(error) }
    }

    override fun finishTool(samples: List<PenSample>) {
        try{
            val mapped=precision.routePen(samples)
            assistedTap?.let{tap->
                assistedTap=null
                if(camera==tap.camera&&assistedCurrent(tap.project,tap.source)){
                    val point=toDocument(mapped.lastOrNull()?:tap.down,tap.camera)
                    val next=appendAssistedPenPrompt(assistedPrompt,point,tap.negative,tap.source)
                    if(next!=null){assistedPrompt=next;assistedSubmittedPrompt=null}
                    else message="This prompt is outside the source or exceeds 64 taps."
                }
                return
            }
            rotationGesture?.let{token->
                rotationGesture=null
                if(precision.inputMatrix()!=rotationPhysical){rotation.cancel();publish();return}
                val point=mapped.lastOrNull()?.let{toDocument(it,token.frame.camera)}
                if(point==null)rotation.cancel()else{rotationDisplay=rotationDisplay?.copy(point=point);rotation.finish(token,point,rotationSnap15)}
                publish();return
            }
            val press=documentPress
            if(press!=null){
                val end=mapped.lastOrNull()?.let{toDocument(it,camera)}?:press.second
                documentPress=null
                if(epoch==documentPressEpoch&&documents.state.value.page==documentPressPage&&documentPressRevision?.sameDocumentRevision(info?:return)==true){
                    val region=Rect(minOf(press.first.x,end.x),minOf(press.first.y,end.y),abs(end.x-press.first.x),abs(end.y-press.first.y))
                    if(documentPressRegion)documents.region(region)else documents.select(region)
                }
                publish();return
            }
            finishMappedTool(mapped)
        }finally{precision.end()}
    }
    private fun finishMappedTool(samples: List<PenSample>) {
        if (eraserPointer) { samples.forEach { erasers.interaction.append(Point(it.x, it.y)) }; eraserPointer = false; erasers.interaction.finish(); return }
        if (selectionPointer) { samples.forEach { selections.interaction.append(Point(it.x, it.y)) }; selectionPointer = false; selections.interaction.finish(); return }
        val active = gesture ?: return
        extendMappedTool(samples)
        if (gesture !== active) return
        gesture = null
        val run = active.stroke
        if (run != null) { run.finished = true; run.prediction = null; run.queue.close(); return }
        if (active.tool == EditorTool.Callout) {
            if (active.revision?.hostSeq == info?.hostSeq && active.revision?.stateHash == info?.stateHash) {
                screen = WorkbenchScreen.Instructions
                val end = active.semanticEnd
                transientShape = null
                val box = semanticDropBox(active.first, end, active.physical)
                semanticEditor.place(active.first, ObjectStyle(EditorDrawing.rgba(active.brush.colorArgb), active.brush.width), active.physical, box, active.semanticModifier)
            } else message = "The document changed. Place the marker again on the current revision."
            publish(); return
        }
        val commands = when (active.tool) {
            EditorTool.Line, EditorTool.Arrow, EditorTool.Rectangle, EditorTool.Ellipse -> transientShape?.let {
                listOf(EditCommand.Create(it.item.objectId, active.layer.id, it.item.shape, it.item.style, it.item.transform))
            } ?: emptyList()
            EditorTool.Eraser -> erased.take(512).map { EditCommand.Delete(it) }
            EditorTool.Select -> active.item?.let { item -> transformed[item.objectId]?.takeIf { it != item.transform }
                ?.let { listOf(EditCommand.SetTransform(item.objectId, it)) } } ?: emptyList()
            else -> emptyList()
        }
        if (commands.isNotEmpty()) edit(commands, transientShape, transformed, erased, gestureId = active.previewId, previewLink = active.link, expected = active.revision, createdAtMs = active.createdAtMs)
        else finishRemotePreview(active.link, active.previewId, true)
        transientShape = null; erased = emptySet(); transformed = emptyMap(); eraserLast = null
        publish()
    }

    override fun cancelTool() { interruptAirInput(); rotationGesture=null;rotation.cancel();clearToolContact(false) }
    private fun clearToolContact(retainPolygon: Boolean) {
        if(rotationGesture!=null){rotationGesture=null;rotation.cancel()}
        documentPress=null;assistedTap=null
        cancelPrecision()
        eraserPointer = false; erasers.interaction.cancelGesture()
        selectionPointer = false; if (!retainPolygon) selections.interaction.cancelGesture()
        gesture?.let { active -> finishRemotePreview(active.link, active.previewId, true) }
        gesture?.stroke?.let { run -> run.canceled.set(true); cancelNative(run.native); run.queue.close(); run.drawing = null }
        gesture = null; transientShape = null; erased = emptySet(); transformed = emptyMap(); eraserLast = null; publish()
    }
    private fun cancelNative(native:WorkbenchStroke?) {
        try { native?.cancel() }
        catch(error:CoreFailure) {
            // A commit can finish before a cancelled/failed refresh is observed.
            // Its terminal state is expected during cleanup, never a new UI error.
            if(error.kind!=CoreFailureKind.Closed&&error.kind!=CoreFailureKind.Cancelled)report(error)
        }
    }
    fun predict(sample: PenSample?) { gesture?.stroke?.prediction = sample?.let{precision.routePen(listOf(it)).single()} }

    private fun batch(run: StrokeRun, samples: List<PenSample>, predicted: Boolean = false): SampleBatch {
        require(samples.isNotEmpty() && samples.size <= if (predicted) 32 else 512)
        require(samples.first().timeMs >= run.lastTime && samples.zipWithNext().all { (a, b) -> a.timeMs <= b.timeMs })
        val points = core.mapPoints(run.camera, true, samples.map { Point(it.x, it.y) })
        return SampleBatch(run.sequence, points.map { it.x }.toDoubleArray(), points.map { it.y }.toDoubleArray(),
            samples.map { require(it.timeMs >= run.start.timeMs && it.timeMs - run.start.timeMs <= UInt.MAX_VALUE.toLong()); (it.timeMs - run.start.timeMs).toUInt() }.toUIntArray(),
            samples.map { it.pressure }.toFloatArray(),
            if (run.capabilities.tilt) samples.map { it.tilt ?: 0f }.toFloatArray() else floatArrayOf(),
            if (run.capabilities.orientation) samples.map { it.orientation ?: 0f }.toFloatArray() else floatArrayOf())
    }

    private fun startStroke(current: WorkbenchProject, run: StrokeRun) {
        scope.launch {
            try {
                mutations.withLock {
                    if (run.canceled.get() || run.epoch != epoch) return@withLock
                    val native = current.beginStroke(run.options); run.native = native
                    if (run.canceled.get()) { cancelNative(native); return@withLock }
                    for (samples in run.queue) {
                        if (run.canceled.get() || run.epoch != epoch) break
                        require(run.samples + samples.size <= 100_000) { "Stroke sample limit reached. The unfinished stroke was canceled." }
                        val realBatch = batch(run, samples)
                        val update = appendWithBackpressure(native, realBatch)
                        try { run.link?.streamStroke(native, realBatch, run.samples.toUInt()) }
                        catch (_: SessionFailure) { /* Durable samples stay in native; disposable prediction is never sent. */ }
                        require(update.firstPolygon == run.polygons) { "Unexpected core geometry sequence" }
                        val additions = withContext(Dispatchers.Default) { EditorDrawing.contourPath(update.contours, run.origin) }
                        run.real.addPath(additions); run.polygons += update.contours.ends.size.toULong()
                        run.samples += samples.size; run.lastTime = samples.last().timeMs; run.sequence++
                        val path = Path(run.real)
                        val predicted = run.prediction
                        if (!run.finished && predicted != null && predicted.timeMs > run.lastTime && predicted.timeMs - run.lastTime <= 100) {
                            try { path.addPath(EditorDrawing.contourPath(native.predict(batch(run, listOf(predicted), true)).contours, run.origin)) }
                            catch (error: CoreFailure) { if (error.kind != CoreFailureKind.Backpressure && error.kind != CoreFailureKind.Cancelled) throw error }
                        }
                        if (!run.canceled.get() && run.epoch == epoch) { run.drawing = wetObject(run, path); publish() }
                    }
                    if (run.finished && !run.canceled.get() && run.epoch == epoch) {
                        // Predictions are replaced by canonical real contours before commit.
                        run.drawing = wetObject(run, Path(run.real)); publish()
                        val result = native.commit()
                        run.committed = true
                        finishRemotePreview(run.link, run.options.gestureId, false)
                        refresh(current, result, run.epoch)
                    } else cancelNative(native)
                }
            } catch (cancel: CancellationException) { cancelNative(run.native); throw cancel }
            catch (error: Exception) { cancelNative(run.native); if (!run.canceled.get()) report(error) }
            finally {
                withContext(NonCancellable) {
                    try { if (!run.committed) finishRemotePreview(run.link, run.options.gestureId, true) }
                    finally {
                        try { run.native?.dispose() }
                        finally { run.native = null; strokes.remove(run); pending--; publish() }
                    }
                }
            }
        }
    }

    private suspend fun appendWithBackpressure(native: WorkbenchStroke, batch: SampleBatch): InkUpdate {
        repeat(4) { attempt ->
            try { return native.append(batch) }
            catch (error: CoreFailure) {
                if (error.kind != CoreFailureKind.Backpressure || attempt == 3) throw error
                delay(4L shl attempt)
            }
        }
        error("Ink queue remained full")
    }

    private fun wetObject(run: StrokeRun, path: Path): DrawObject {
        val item = RenderItem(run.options.objectId, run.layerId, run.opacity, run.blend, Rect(0.0, 0.0, 0.0, 0.0),
            Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(), ObjectStyle(run.options.rgba, run.options.width), Shape.Stroke(run.options.family), false)
        return DrawObject(item, run.origin, path, true)
    }
    private fun shape(tool: EditorTool, a: Point, b: Point): Shape? {
        if (hypot(a.x - b.x, a.y - b.y) < .1) return null
        val rect = Rect(min(a.x, b.x), min(a.y, b.y), max(.1, abs(a.x - b.x)), max(.1, abs(a.y - b.y)))
        return when (tool) {
            EditorTool.Line -> Shape.Line(listOf(a, b)); EditorTool.Arrow -> Shape.Arrow(listOf(a, b))
            EditorTool.Rectangle -> Shape.Rectangle(rect); EditorTool.Ellipse -> Shape.Ellipse(rect)
            else -> null
        }
    }
    private fun previewShape(shape: Shape, active: ToolGesture): DrawObject {
        return EditorDrawing.objectPath(RenderItem(checkNotNull(active.newObjectId), active.layer.id,
            active.layer.opacity, active.layer.blend, Rect(0.0, 0.0, 0.0, 0.0), Contours(longArrayOf(), longArrayOf(), uintArrayOf()),
            Transform(), ObjectStyle(EditorDrawing.rgba(active.brush.colorArgb), active.brush.width), shape, false))
    }

    private fun newId() = core.newId(System.currentTimeMillis().toULong())
    private fun editOptions(count: Int): EditOptions = EditOptions(newId(), checkNotNull(document).documentId, repository.deviceId,
        repository.lamport(count), System.currentTimeMillis())
    private fun edit(commands: List<EditCommand>, preview: DrawObject? = null, transforms: Map<String, Transform> = emptyMap(), hidden: Set<String> = emptySet(),
                     succeeded: (() -> Unit)? = null, gestureId: String? = null, previewLink: ProjectLink? = null, expected: ProjectInfo? = null,
                     createdAtMs: Long = System.currentTimeMillis()) {
        val current = project ?: return
        if (commands.isEmpty()) return
        if(rotationUnavailable()||workflows.blocksInput()||imageFrames.state.value.blocksInput){finishRemotePreview(previewLink,gestureId,true);message="Finish or close the current controls before editing.";return}
        if (operationCount >= 32) { finishRemotePreview(previewLink, gestureId, true); message = "The edit queue is full. Try the edit again after saving finishes."; return }
        val token = epoch
        val options = editOptions(commands.sumOf { when (it) { is EditCommand.SetText -> 3; is EditCommand.SetStyle -> 2; else -> 1 } })
            .copy(gestureId = gestureId, expectedHostSeq = expected?.hostSeq, expectedStateHash = expected?.stateHash, createdAtMs = createdAtMs)
        val first = commands.firstOrNull() as? EditCommand.Create
        if (preview != null && first != null) pendingPreviews[options.transactionId] = preview.copy(item = preview.item.copy(objectId = first.objectId))
        if (transforms.isNotEmpty()) pendingTransforms[options.transactionId] = transforms.toMap()
        if (hidden.isNotEmpty()) pendingErases[options.transactionId] = hidden.toSet()
        invalidateExport(); pending++; operationCount++
        scope.launch {
            var committed = false
            try { mutations.withLock { if (token == epoch) {
                val result = current.edit(options, commands); committed = true
                finishRemotePreview(previewLink, gestureId, false)
                refresh(current, result, token); succeeded?.invoke()
            } } }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { report(error) }
            finally {
                if (!committed) finishRemotePreview(previewLink, gestureId, true)
                pending--; operationCount--; pendingPreviews.remove(options.transactionId)
                pendingTransforms.remove(options.transactionId); pendingErases.remove(options.transactionId); publish()
            }
        }
    }
    private suspend fun refresh(current: WorkbenchProject, revision: ProjectInfo, token: Long) {
        refreshMutex.withLock {
        if (token != epoch || current !== project || disposing || revision.projectId != info?.projectId) return
        val ticket = refreshAdmission.capture()
        val doc = document ?: return
        val fetched = documentRefresh.load(doc,{document}){current.document(doc.documentId)} ?: return
        val next = fetched.snapshot
        val drawing = preparedDrawing(next.render.items)
        if (token != epoch || project !== current || disposing || !refreshAdmission.accepts(ticket) ||
            !documentRefresh.accepts(fetched.ticket,document,next)) return
        // The document and its own revision are one native snapshot. Host sequence
        // alone cannot order optimistic edits sharing the same accepted base.
        if (document?.render?.revision != next.render.revision) clearPeerObjects()
        val priorComposition=composedBackground!=null
        if(document?.render?.revision!=next.render.revision && (priorComposition || next.hasToneAdjustments)){
            background=null;composedBackground=null
        }
        info = next.render.revision
        if(giantMode||giantOwner.retaining)giantOwner.invalidate()
        document = next; objects = drawing
        documentOwner.bind(current,next)
        bindWorkflows()
        if(priorComposition || next.hasToneAdjustments)publish()
        refreshOriginalFormatNotice()
        if(giantMode||giantDisplayRequired(next)||next.hasToneAdjustments||priorComposition)loadBackground()
        instructionEditor.refresh(); instructionFocus.revisionChanged(); semanticEditor.refresh()
        if (exportReady?.matches(info, document) != true) exportReady = null
        if (exportPreflight?.revision?.stateHash != info?.stateHash || exportPreflight?.revision?.hostSeq != info?.hostSeq) {
            exportPreflight = null; checkedExport = null; checkedExportIntent = null
        }
        if (objects.none { it.item.objectId == selected }) selected = null
        publish()
        }
    }
    fun saveText(text: String, font: String, size: Double) {
        if (busy || pending != 0) return
        val draft = textDraft ?: return
        if (text.isBlank() || text.length > 16_384 || font !in listOf("Inter", "Noto Sans", "Noto Sans Mono") || !size.isFinite() || size !in 4.0..512.0) {
            message = "Enter text up to 16,384 characters and choose a size from 4 to 512 pixels."; return
        }
        busy = true
        scope.launch {
            try {
                // Validate bundled glyph coverage before dismissing the editor.
                // Failed text remains available for correction or copying.
                core.layoutText(text, font, size.toFloat())
                val commands = if (draft.objectId != null) listOf(EditCommand.SetText(draft.objectId, text, font, size))
                else listOf(EditCommand.Create(newId(), selectedLayer().id, Shape.Text(draft.anchor, text, font, size),
                    ObjectStyle(EditorDrawing.rgba(preferences.brush(tool).colorArgb), preferences.brush(tool).width)))
                edit(commands, succeeded = { textDraft = null })
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: CoreFailure) { message = "The bundled fonts could not lay out this text. Try another bundled font or remove unsupported characters. Your draft is retained." }
            catch (error: Exception) { report(error) }
            finally { busy = false }
        }
    }
    fun dismissText() { textDraft = null }
    fun editSelectedText() {
        val item = objects.firstOrNull { it.item.objectId == selected }?.item ?: return
        val text = item.shape as? Shape.Text ?: return
        textDraft = TextDraft(item.objectId, text.anchor, text.text, text.font, text.size)
    }
    fun deleteSelected() {
        selected?.let { id ->
            if (objects.any { it.item.objectId == id && it.item.shape is Shape.Marker }) {
                screen = WorkbenchScreen.Instructions; instructionEditor.deleteMarker(id)
            } else edit(listOf(EditCommand.Delete(id)))
        }
        showContext = false
    }

    override fun undo() = undoRedo(false)
    override fun redo() = undoRedo(true)
    private fun undoRedo(redo: Boolean) {
        if (busy || workflows.blocksInput() || imageFrames.state.value.blocksInput) return
        cancelTool()
        if (pending != 0) { message = "Wait for the current edit to finish before undo or redo."; return }
        val current = project ?: return
        if (if (redo) info?.canRedo != true else info?.canUndo != true) return
        val options = editOptions(512); val token = epoch; invalidateExport(); pending++
        scope.launch {
            try { mutations.withLock { commitUndoRedo(current, token, options, redo) } }
            catch (error: Exception) { report(error) }
            finally { pending-- }
        }
    }
    private suspend fun commitUndoRedo(current: WorkbenchProject, token: Long, options: EditOptions, redo: Boolean) {
        if (token == epoch && current === project) refresh(current, current.undoRedo(options, redo), token)
    }

    override fun pan(dx: Double, dy: Double) {
        if (gesture != null) return
        followPeer = false
        invalidateViewCheck()
        val middle = Point(camera.viewportWidth / 2, camera.viewportHeight / 2)
        val next = core.mapPoints(camera, true, listOf(Point(middle.x - dx, middle.y - dy))).single()
        camera = camera.copy(center = next); publish()
    }
    override fun pinch(previousX: Double, previousY: Double, nextX: Double, nextY: Double, scale: Double, radians: Double) {
        if (gesture != null || !scale.isFinite() || scale <= 0 || !radians.isFinite()) return
        followPeer = false
        invalidateViewCheck()
        val anchored = core.mapPoints(camera, true, listOf(Point(previousX, previousY))).single()
        val raw = unsnappedRotation + radians
        unsnappedRotation = raw
        val cardinal = round(raw / (Math.PI / 2)) * (Math.PI / 2)
        val rotation = if (abs(raw - cardinal) <= Math.PI / 36) cardinal else raw
        val next = camera.copy(scale = (camera.scale * scale).coerceIn(.001, 128.0), rotation = rotation)
        val under = core.mapPoints(next, true, listOf(Point(nextX, nextY))).single()
        camera = next.copy(center = Point(next.center.x + anchored.x - under.x, next.center.y + anchored.y - under.y)); publish()
    }
    override fun contextMenu(x: Double, y: Double) {
        cancelTool()
        selected = scene?.let { EditorDrawing.hit(it, Point(x, y), 8 * density)?.item?.objectId }
        showContext = true; publish()
    }
    override fun hover(sample: PenSample?, capabilities: StylusCapabilities) {
        this.capabilities = capabilities
        hoverPoint = sample?.let { Point(it.x, it.y) };precision.hover(hoverPoint);publish()
    }

    private fun previewBrush() {
        previewJob?.cancel(); cancelNative(previewHandle); previewHandle = null; brushPreview = null
        val current = project ?: return
        val layer = try { selectedLayer() } catch (_: Exception) { return }
        val brush = displayedBrush()
        val family = when (tool) { EditorTool.Highlighter -> "highlighter"; EditorTool.Marker -> "marker"; else -> "pen" }
        previewJob = scope.launch {
            var native: WorkbenchStroke? = null
            try {
                delay(40)
                val now = System.currentTimeMillis()
                native = current.beginStroke(StrokeOptions(newId(), newId(), newId(), checkNotNull(document).documentId, layer.id,
                    repository.deviceId, repository.lamport(3), now, family, min(brush.width, 48.0), EditorDrawing.rgba(brush.colorArgb),
                    brush.stabilization, brush.pressure.values()))
                previewHandle = native
                val count = 64
                val batch = SampleBatch(1uL, DoubleArray(count) { 12.0 + it * 296.0 / (count - 1) },
                    DoubleArray(count) { 36.0 + sin(it * .23) * 12.0 }, UIntArray(count) { (it * 8).toUInt() }, FloatArray(count) { .05f + it * .95f / (count - 1) })
                val result = native.append(batch)
                brushPreview = withContext(Dispatchers.Default) { EditorDrawing.contourPath(result.contours) }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { if (showBrush) report(error) }
            finally {
                withContext(NonCancellable) {
                    try { cancelNative(native) }
                    finally {
                        try { native?.dispose() }
                        finally { if (previewHandle === native) previewHandle = null }
                    }
                }
            }
        }
    }

    private fun exportOptions(doc: DocumentSnapshot, selection: ExportSelection): ExportOptions {
        val region = if (selection.viewOnly) viewExportRegion(core.mapPoints(camera, true, listOf(
            Point(0.0, 0.0), Point(camera.viewportWidth, 0.0), Point(camera.viewportWidth, camera.viewportHeight), Point(0.0, camera.viewportHeight))), doc.width, doc.height) else null
        require(selection.quality in 1..100)
        return ExportOptions(doc.documentId, format = selection.encoding.format(selection.quality),
            marked = selection.marked, region = region, convertToSrgb = selection.convertToSrgb,
            matteRgb = selection.matteRgb,
            allowDepthReduction = selection.allowDepthReduction, assumeUntaggedSrgb = approvedAssumption(doc))
    }
    fun prepareExport() {
        if (busy || pending != 0 || imageFrames.state.value.blocksInput || workflows.blocksInput()) return
        val current = project ?: return
        val doc = document ?: return
        val selection = exportSelection
        val view = camera
        val source = try { exportOptions(doc, selection) } catch (error: Exception) { report(error); return }
        val revision = doc.render.revision
        val binding = WorkflowBinding(revision.projectId, doc.documentId, revision.hostSeq, revision.stateHash)
        val token = epoch
        busy = true; cancelTool(); invalidateExport()
        val job = scope.launch {
            try {
                transferLabel = "Checking image dimensions, color and export limits…"
                val checked = mutations.withLock {
                    if (project !== current || epoch != token || document !== doc || exportSelection != selection ||
                        (selection.viewOnly && camera != view) || info?.stateHash != binding.stateHash || info?.hostSeq != binding.hostSeq)
                        throw WorkflowFailure(WorkflowFailureKind.Stale)
                    val (projected, geometry) = projectExportOptions(current, binding, source)
                    val intent = ExportCallerIntent(current, token, doc.page, view, selection, binding, source, projected, geometry)
                    if (!intent.matches(project, epoch, document, info, camera, exportSelection))
                        throw WorkflowFailure(WorkflowFailureKind.Stale)
                    val result = repository.preflight(current, projected)
                    if (result.width != geometry.encodedWidth || result.height != geometry.encodedHeight ||
                        result.revision.projectId != binding.projectId || result.revision.hostSeq != binding.hostSeq ||
                        result.revision.stateHash != binding.stateHash) throw WorkflowFailure(WorkflowFailureKind.Stale)
                    intent to result
                }
                if (checked.first.matches(project, epoch, document, info, camera, exportSelection)) {
                    checkedExportIntent = checked.first; checkedExport = checked.first.options; exportPreflight = checked.second
                } else message = "The project changed during the check. Check the export again."
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: WorkflowFailure) { message = "The project or output frame changed. Check this export again." }
            catch (error: Exception) { report(error) }
            finally { busy = false; transferLabel = null; transferJob = null }
        }
        transferJob = job.takeIf { it.isActive }
    }
    fun exportTicket(): ExportTicket? {
        if (pickerTicket != null) { message = "Finish or cancel the pending file destination before starting another export."; return null }
        val current = info ?: return null
        val options = checkedExport ?: return null
        val intent = checkedExportIntent ?: return null
        val checked = exportPreflight ?: return null
        if (busy || pending != 0 || imageFrames.state.value.blocksInput || workflows.blocksInput() ||
            checked.revision.stateHash != current.stateHash || checked.revision.hostSeq != current.hostSeq ||
            !intent.matches(project, epoch, document, info, camera, exportSelection) || options != intent.options ||
            checked.width != intent.geometry.encodedWidth || checked.height != intent.geometry.encodedHeight) return null
        return ExportTicket(newId(), current.projectId, options.documentId, current.hostSeq, current.stateHash, options, exportSelection.encoding).also { pickerTicket = it }
    }
    fun cancelExportTicket(id: String?) { if (pickerTicket?.id == id) pickerTicket = null }
    fun export(uri: Uri, ticketId: String?) {
        val ticket = pickerTicket?.takeIf { it.id == ticketId }
        cancelExportTicket(ticketId)
        if (uri.scheme != "content") { message = "The export destination must be a document provider."; return }
        if (disposing || busy || pending != 0 || imageFrames.state.value.blocksInput || workflows.blocksInput() ||
            project == null || document == null || checkedExport == null ||
            ticket?.matches(info, document, checkedExport) != true ||
            checkedExportIntent?.matches(project, epoch, document, info, camera, exportSelection) != true) {
            message = "The project or export settings changed while choosing a destination. Check the export again."
            scope.launch { try { repository.discardDestination(uri) } catch (error: Exception) { report(error) } }
            return
        }
        exportImage(uri, sharing = false)
    }
    fun sharePng() = exportImage(null, sharing = true)
    private fun exportImage(uri: Uri?, sharing: Boolean) {
        if (busy || pending != 0 || imageFrames.state.value.blocksInput || workflows.blocksInput()) return
        val current = project ?: return
        val doc = document ?: return
        val intent = checkedExportIntent ?: run { message = "Check the export settings before saving or sharing."; return }
        val options = checkedExport ?: run { message = "Check the export settings before saving or sharing."; return }
        if (sharing && options.format != ImageFormat.Png8 && options.format != ImageFormat.Png16) {
            message = "Choose PNG 8-bit or PNG 16-bit to share an image."; return
        }
        if (exportPreflight?.revision?.stateHash != info?.stateHash || exportPreflight?.revision?.hostSeq != info?.hostSeq ||
            options != intent.options || !intent.matches(current, epoch, doc, info, camera, exportSelection)) {
            invalidateExport(); message = "The project or output frame changed. Check the export again."; return
        }
        if (doc.bitDepth > 8u && options.format == ImageFormat.Png8 && !options.allowDepthReduction) {
            message = "Choose PNG 16-bit or explicitly allow reduction to 8-bit."; return
        }
        exportReady = null
        busy = true; cancelTool()
        val job = scope.launch {
            var pendingShare: MediaHandoff.Lease? = null
            val validateReceipt: suspend (FileExportResult) -> Unit = { result ->
                if (!intent.matches(project, epoch, document, info, camera, exportSelection) ||
                    checkedExportIntent !== intent || checkedExport != options)
                    throw WorkflowFailure(WorkflowFailureKind.Stale)
                checkedAndroidExportReceipt(result, intent)
            }
            try {
                val result = mutations.withLock {
                    if (!intent.matches(project, epoch, document, info, camera, exportSelection))
                        throw WorkflowFailure(WorkflowFailureKind.Stale)
                    if (sharing) {
                        val shared = repository.sharePng(current, options, validateReceipt) { transferLabel = it }
                        pendingShare = shared.first
                        shared.second
                    } else repository.exportImage(checkNotNull(uri), current, options, validateReceipt) { transferLabel = it }
                }
                if (!intent.matches(project, epoch, document, info, camera, exportSelection) ||
                    result.revision.projectId != intent.binding.projectId ||
                    result.revision.hostSeq != intent.binding.hostSeq || result.revision.stateHash != intent.binding.stateHash)
                    throw WorkflowFailure(WorkflowFailureKind.Stale)
                exportReady = ExportReady(result.revision.projectId, doc.documentId, result.revision.stateHash,
                    result.revision.hostSeq, intent.geometry.encodedWidth, intent.geometry.encodedHeight, result.blake3)
                if (sharing) {
                    shareLease = pendingShare
                    shareUri = pendingShare?.uri
                    pendingShare = null
                }
                message = "${exportSelection.encoding.label} exported at revision ${result.revision.hostSeq}. The original image is unchanged."
            } catch (cancel: CancellationException) {
                message = "Export canceled. The original and saved edits are unchanged."
                throw cancel
            } catch (error: WorkflowFailure) {
                message = "The project or output frame changed. Check the export again."
            } catch (error: CoreFailure) {
                message = rasterFailure(error, importing = false)
            } catch (error: Exception) { report(error) }
            finally {
                withContext(NonCancellable) {
                    pendingShare?.let { cleanupTransfer { MediaHandoff(getApplication()).remove(it, camera = false) } }
                }
                busy = false; transferLabel = null; transferJob = null
            }
        }
        transferJob = job.takeIf { it.isActive }
    }
    fun cancelTransfer() {
        if (transferJob != null && transferJob === incomingProducer) explicitImportCancel = true
        transferLabel = "Canceling and removing temporary files…"; transferJob?.cancel()
    }
    private fun rasterFailure(error: CoreFailure, importing: Boolean): String = when (error.kind) {
        CoreFailureKind.ImageCodecUnavailable -> "A compatible image decoder is unavailable on this device. The original is preserved."
        CoreFailureKind.Backpressure -> if (importing)
            "This image exceeds the current decoder, memory or scratch-storage limit. PNG and baseline JPEG have bounded import paths; some progressive JPEG and WebP images still need a larger decoder budget. The original was not resized or changed."
        else "This export exceeds the current memory, scratch-storage or output-size limit. Large result images and unusually complex marks can still exceed the bounded PNG path. The original and edits are intact."
        CoreFailureKind.Unsupported -> "This image uses an unsupported format or feature, or exceeds 50 megapixels. PNG and baseline JPEG are supported within the current limits; no smaller replacement was created."
        CoreFailureKind.Storage -> "The image transfer could not finish writing its files. The original and saved edits are intact."
        CoreFailureKind.Cancelled -> "The image transfer was canceled."
        else -> "The image transfer could not finish safely. The original and saved edits are intact."
    }
    fun drawCallback(inputMs: Long) {
        drawCallbacks++; lastDrawCallbackMs = max(0, android.os.SystemClock.uptimeMillis() - inputMs)
    }
    private fun report(error: Exception) {
        if (error is CancellationException) return
        message = when (error) {
            is DocumentFailure -> documentFailureText(error)
            is TransferFailure -> when (error.kind) {
                TransferFailureKind.PixelLimit -> "Images over 50 megapixels aren't supported yet. The original was not resized or changed."
                TransferFailureKind.Dimensions -> "This format cannot represent the requested dimensions. Choose PNG or a smaller region; the original is unchanged."
                TransferFailureKind.Memory, TransferFailureKind.Backpressure -> "This export exceeds the current memory budget. Try PNG or a smaller region; no resizing was applied."
                TransferFailureKind.Alpha -> "JPEG cannot preserve transparency. Explicitly choose a white background or use PNG/WebP."
                TransferFailureKind.Depth -> "This format reduces the original bit depth. Choose PNG 16-bit or explicitly allow 8-bit output."
                TransferFailureKind.Metadata -> "The color profile could not be preserved or converted safely. The original is unchanged."
                TransferFailureKind.EncodedLimit -> "The encoded image exceeds the current output limit. Choose a smaller region or another format."
                TransferFailureKind.ScratchLimit -> "This transfer exceeds its temporary-storage limit. The original and edits are intact."
                TransferFailureKind.Cancelled -> "The image transfer was canceled."
                TransferFailureKind.ImageCodecUnavailable -> "An installed AVIF decoder is unavailable. The original is unchanged."
                TransferFailureKind.Unsupported -> "This image or export feature is not supported by the current decoder. The original is unchanged."
                else -> "The image transfer could not finish safely. The original and edits are intact."
            }
            is ImageTooLarge, is RasterTransferFailure -> error.message
            is GiantImageFailure -> when (error.kind) {
                GiantImageFailureKind.Limit, GiantImageFailureKind.Memory, GiantImageFailureKind.Backpressure ->
                    "This large JPEG exceeds the available tile, memory or scratch-storage allowance. Its source was not reduced."
                GiantImageFailureKind.Unsupported -> "This device's tiled producer supports JPEG originals only. Choose regular import for a supported smaller image."
                GiantImageFailureKind.Storage -> "The large JPEG could not be saved safely. Check free space and try again."
                GiantImageFailureKind.Cancelled -> "Large-JPEG import was canceled."
                else -> "The large JPEG could not be verified or opened. Existing projects are unchanged."
            }
            is CoreFailure -> when (error.kind) {
                CoreFailureKind.Backpressure -> "The core is at its resource limit. The unfinished action was canceled; saved work is intact."
                CoreFailureKind.Storage -> "The edit could not be saved. Check available storage and try again."
                CoreFailureKind.ImageCodecUnavailable -> "An installed AVIF decoder is unavailable. The original is unchanged."
                CoreFailureKind.Cancelled -> "The unfinished action was canceled."
                else -> "The core could not complete this action (${error.kind}). Saved work is intact."
            }
            is IllegalArgumentException, is IllegalStateException, is UnsupportedOperationException -> error.message ?: "This action could not be completed."
            else -> "This action could not be completed. Saved work is intact."
        }
    }
    override fun onCleared() {
        interruptAirInput(clearView = true, clearBarrel = true); resetAirRemoteMotion(); airActionRouter.close(); airDispatcher.close()
        imageFrames.beginDeparture();workflows.beginDeparture();rotation.cancel()
        giantOwner.invalidate()
        disposing = true
        invalidateAssisted()
        remoteCapability?.let{it.seal("background");retireRemoteEditLater(it)}
        captureOwner.close()
        importReadiness.close()
        incomingJob?.cancel()
        transferJob?.cancel()
        cancelTool(); diagnostics.background(); previewJob?.cancel(); cancelNative(previewHandle)
        // Close after accepted writes already queued in the native worker. The
        // retained ViewModel survives activity recreation; this is final disposal.
        preferencesQueue.close()
        scope.launch {
            try { preferencesWriter.join(); diagnostics.awaitSave() }
            finally {
                // Resolve the current handle only after any in-flight loader has
                // finished under this same mutex. Capturing it before waiting
                // can miss a project returned while the ViewModel is clearing.
                try {
                    incomingJob?.join()
                    try { connectionJob?.cancelAndJoin(); captureOpenWork?.cancelAndJoin(); closeAssisted();mutations.withLock { closeCurrent();imageFrames.close();workflows.close();rotation.close();documentOwner.close();closePrecision() } }
                    finally {
                        try { pairing.close() }
                        finally {
                            shareLease?.let { lease -> cleanupTransfer { MediaHandoff(getApplication()).remove(lease, camera = false) } }
                            shareLease = null; shareUri = null
                        }
                    }
                } catch (error: Exception) {
                    report(error)
                    DocumentRetirementRegistry.retain(this@EditorController){closeAssisted();mutations.withLock{closeCurrent();imageFrames.close();workflows.close();rotation.close();documentOwner.close();closePrecision()}}
                } finally { scope.cancel() }
            }
        }
        super.onCleared()
    }
}

private fun PressureCurve.values() = listOf(0.0, 0.0, x1.toDouble(), y1.toDouble(), x2.toDouble(), y2.toDouble(), 1.0, 1.0)
