@file:OptIn(ExperimentalUnsignedTypes::class)

package com.visualworkbench.android.editor

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Path
import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import com.visualworkbench.android.diagnostics.DiagnosticsController
import com.visualworkbench.android.input.*
import com.visualworkbench.android.ui.ProjectCard
import com.visualworkbench.shared.*
import com.visualworkbench.android.session.PairingController
import com.visualworkbench.android.session.RefreshAdmission
import com.visualworkbench.android.session.sessionMessage
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.*

internal enum class WorkbenchScreen { Projects, Canvas, Settings, Diagnostics, Pairing }
internal data class TextDraft(val objectId: String?, val anchor: Point, val text: String = "", val font: String = "Inter", val size: Double = 32.0)
internal enum class ImportOutcome { Imported, Cancelled, Retained }
internal data class CameraCapture(val token: String, val bytes: Long)

/** UI state lives on Main; native/storage work is suspendable and input queues are bounded. */
internal class EditorController @JvmOverloads constructor(application: Application, val core: WorkbenchCore = workbenchCore(),
    sessionFactory: suspend (android.content.Context, String) -> WorkbenchSessions = ::createAndroidSessions) : AndroidViewModel(application), CanvasGestureSink {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val repository = ProjectRepository(application, core)
    private val preferencesStore = EditorPreferencesStore(application)
    private val preferencesQueue = Channel<EditorPreferences>(Channel.CONFLATED)
    private val mutations = Mutex()
    private val refreshMutex = Mutex()
    private val refreshAdmission = RefreshAdmission()
    private var changesJob: Job? = null
    private var refreshJob: Job? = null
    private var refreshQueue: Channel<Unit>? = null
    val pairing = PairingController(application, scope, { repository.deviceId }, { message = it }, sessionFactory)
    private var link: ProjectLink? = null
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
    var screen by mutableStateOf(WorkbenchScreen.Projects); private set
    var preferences by mutableStateOf(EditorPreferences()); private set
    var tool by mutableStateOf(EditorTool.Pen); private set
    var projects by mutableStateOf<List<ProjectCard>>(emptyList()); private set
    // Native-operation admission must not wait for a Compose host or
    // global snapshot apply notifications. UI state remains observable.
    private val importReadiness = EditorReadiness()
    private var busyState by mutableStateOf(true)
    var busy: Boolean
        get() = busyState
        private set(value) { busyState = value; importReadiness.busy(value) }
    var transferLabel by mutableStateOf<String?>(null); private set
    private var pendingState by mutableStateOf(0)
    var pending: Int
        get() = pendingState
        private set(value) { pendingState = value; importReadiness.pending(value) }
    var message by mutableStateOf<String?>(null); private set
    var blockingMessage by mutableStateOf<String?>(null); private set
    var needsColorConsent by mutableStateOf(false); private set
    var info by mutableStateOf<ProjectInfo?>(null); private set
    var document by mutableStateOf<DocumentSnapshot?>(null); private set
    var selected by mutableStateOf<String?>(null); private set
    var textDraft by mutableStateOf<TextDraft?>(null); private set
    var showContext by mutableStateOf(false)
    var showBrush by mutableStateOf(false); private set
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
    private var pickerTicket: ExportTicket? = null
    private var project: WorkbenchProject? = null
    private var transferJob: Job? = null
    private var incomingJob: Job? = null
    private var incomingProducer: Job? = null
    private var explicitImportCancel = false
    private var shareLease: MediaHandoff.Lease? = null
    private var disposing = false
    private var epoch = 0L
    private var sceneRevision = 0L
    private var background: Bitmap? = null
    private var objects = emptyList<DrawObject>()
    private var camera = Camera(Point(800.0, 600.0), 1.0, 0.0, 1.0, 1.0)
    private var cameraFitted = false
    private var unsnappedRotation = 0.0
    private var density = 1f
    private var hoverPoint: Point? = null
    private var transformed = emptyMap<String, Transform>()
    private var erased = emptySet<String>()
    private var transientShape: DrawObject? = null
    private var gesture: ToolGesture? = null
    private val strokes = LinkedHashSet<StrokeRun>()
    private var previewJob: Job? = null
    private var previewHandle: WorkbenchStroke? = null
    private var colorAssumed = false
    private var canvasSink: ((CanvasScene) -> Unit)? = null
    private var operationCount = 0
    private var eraserLast: Point? = null
    private var selectedStyleDraft by mutableStateOf<ObjectStyle?>(null)
    private val pendingPreviews = LinkedHashMap<String, DrawObject>()
    private val pendingTransforms = LinkedHashMap<String, Map<String, Transform>>()
    private val pendingErases = LinkedHashMap<String, Set<String>>()
    private val preferencesWriter: Job

    init {
        preferencesWriter = scope.launch {
            for (settings in preferencesQueue) try { preferencesStore.write(settings) }
            catch (_: Exception) { message = "Settings could not be saved. Try changing the setting again." }
        }
        scope.launch {
            try { repository.initialize(); preferences = preferencesStore.read(); refreshProjects(); refreshCaptures() }
            catch (error: Exception) { report(error) }
            finally { busy = false }
        }
    }

    fun attachCanvas(sink: (CanvasScene) -> Unit) { canvasSink = sink; scene?.let(sink) }
    fun detachCanvas() { canvasSink = null; cancelTool(); hoverPoint = null }
    fun dismissMessage() { message = null }
    fun transferMessage(value: String) { message = value }
    fun configureExport(value: ExportSelection) { if (exportSelection != value) { exportSelection = value; exportPreflight = null; checkedExport = null } }
    private fun invalidateExport() { exportReady = null; exportPreflight = null; checkedExport = null }
    private fun invalidateViewCheck() { if (exportSelection.viewOnly) { exportPreflight = null; checkedExport = null } }
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
        preferences = value; preferencesQueue.trySend(value); publish()
    }
    fun chooseTool(value: EditorTool) { cancelTool(); tool = value; showContext = false; hoverPoint = null; publish() }
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
        if (transferJob != null) { message = "Finish or cancel the image transfer before leaving this screen."; return }
        cancelTool(); diagnostics.background(); showContext = false
        if (screen == WorkbenchScreen.Pairing && next != WorkbenchScreen.Pairing) pairing.leave()
        if (next == WorkbenchScreen.Pairing) { pairing.scanner = true; pairing.enter() }
        if (next == WorkbenchScreen.Projects && project != null) {
            if (pending != 0) { message = "Finishing the current edit. Try returning to projects in a moment."; return }
            busy = true
            scope.launch {
                try { mutations.withLock { closeCurrent() }; refreshProjects(); screen = next }
                catch (error: Exception) { report(error) }
                finally { busy = false }
            }
        } else screen = next
    }
    fun returnFromSettings() { screen = if (project == null) WorkbenchScreen.Projects else WorkbenchScreen.Canvas }
    fun onBackground() {
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
    fun importImage(uri: Uri) = openRequest(transfer = true) { repository.importImage(uri) { transferLabel = it } }
    /** Copy transient picker/share grants while this process owns them; no persistent grant is needed. */
    fun receiveImage(uri: Uri, cleanup: suspend (ImportOutcome) -> Unit = {}) {
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
                    importer = openRequest(transfer = true) {
                        val handle = repository.importImage(uri) { transferLabel = it }
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
    private fun openRequest(knownSrgb: Boolean = false, transfer: Boolean = false, load: suspend () -> WorkbenchProject): Job? {
        if (disposing || busy || pending != 0) return null
        busy = true; cancelTool()
        val job = scope.launch {
            try {
                val opened=mutations.withLock {
                    if(disposing)return@withLock false
                    closeCurrent()
                    val next=load()
                    if(disposing){next.close();false}else{project=next;true}
                }
                if(!opened)return@launch
                val current = checkNotNull(project)
                val token = epoch
                info = current.info()
                val doc = info?.documentIds?.firstOrNull() ?: error("This project has no image document")
                val snapshot = current.document(doc)
                val drawing = withContext(Dispatchers.Default) { snapshot.render.items.map(EditorDrawing::objectPath) }
                if (disposing || token != epoch || current !== project) return@launch
                document = snapshot; info = snapshot.render.revision; objects = drawing
                observeProject(current, token)
                colorAssumed = knownSrgb; screen = WorkbenchScreen.Canvas; cameraFitted = false
                fit(); loadBackground()
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: CoreFailure) {
                if (transfer) message = rasterFailure(error, importing = true) else report(error)
            }
            catch (error: Exception) { report(error) }
            finally { busy = false; if (transfer) { transferLabel = null; transferJob = null }; publish() }
        }
        if (transfer) transferJob = job.takeIf { it.isActive }
        return job
    }

    private suspend fun closeCurrent() {
        withContext(NonCancellable) {
        exportReady = null
        exportPreflight = null; checkedExport = null
        epoch++; refreshAdmission.reset()
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
                background = null; objects = emptyList(); selected = null; transientShape = null
                transformed = emptyMap(); erased = emptySet(); scene = null; blockingMessage = null; needsColorConsent = false
            }
        }
        // Published render snapshots may still reference the old bitmap. Allow
        // normal reachability to release it instead of recycling under a draw.
        }
    }

    private fun observeProject(current: WorkbenchProject, token: Long) {
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
        busy = true
        connectionJob = scope.launch {
            try {
                mutations.withLock {
                    if (disposing || project !== current) return@withLock
                    detachLink()
                    val sessions = pairing.service()
                    val role = sessions.projectRole(current)
                    if (!role.isHost && role.hostDeviceId != peer) {
                        message = "This project belongs to a different host. Choose its paired computer."; return@withLock
                    }
                    val created = if (role.isHost) sessions.host(current, peer, endpoints) else sessions.connect(current, peer, endpoints)
                    if (disposing || project !== current) { withContext(NonCancellable) { created.close() }; return@withLock }
                    link = created; observeLink(created, current, epoch, ++linkEpoch)
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
        val job = openRequest(transfer = true) {
            transferLabel = "Receiving and verifying the computer's project…"
            pairing.service().receiveProject(repository.receivePath(), peer, endpoints)
        } ?: return
        scope.launch {
            job.join()
            if (!disposing && project != null && document != null && screen == WorkbenchScreen.Canvas) connectCurrent()
        }
    }

    private fun observeLink(created: ProjectLink, current: WorkbenchProject, token: Long, generation: Long) {
        clearPeerObjects()
        connection = created.status(); lastViewport = null
        linkChanges = scope.launch {
            try {
                created.changes.collect { status ->
                    if (token != epoch || generation != linkEpoch || project !== current || link !== created) return@collect
                    if (connection?.let { status.sequence <= it.sequence } == true) return@collect
                    val before = connection
                    connection = status
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
                            val drawing = withContext(Dispatchers.Default) { preview.items.map(EditorDrawing::objectPath) }
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
        withContext(NonCancellable) {
        linkEpoch++; followPeer = false
        linkChanges?.cancelAndJoin(); linkChanges = null
        peerWorker?.cancelAndJoin(); peerWorker = null
        val previous = link; link = null
        connection = null; clearPeerObjects(); lastViewport = null
        if (previous != null) withContext(NonCancellable) { previous.close() }
        }
    }
    fun disconnect() {
        if (busy) return
        cancelTool(); scope.launch { try { detachLink(); publish() } catch (error: Exception) { report(error) } }
    }
    fun revoke(peer: String) { pairing.revoke(peer) { cancelTool(); detachLink(); publish() } }
    fun matchPeer() { followPeer = false; matchPeerCamera(); publish() }
    fun followPeer(enabled: Boolean) { followPeer = enabled; if (enabled) matchPeerCamera(); publish() }
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

    private suspend fun loadBackground() {
        val current = project ?: return
        val doc = document ?: return
        val token = epoch
        try {
            val image = current.background(doc.documentId, assumeUntaggedSrgb = colorAssumed)
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
            if (token != epoch) return
            background = bitmap; blockingMessage = null; needsColorConsent = false
        } catch (cancel: CancellationException) { throw cancel }
        catch (error: CoreFailure) {
            if (token != epoch) return
            needsColorConsent = error.kind == CoreFailureKind.Raster && !colorAssumed
            blockingMessage = if (error.kind == CoreFailureKind.Backpressure)
                "This image exceeds the current full-resolution display memory budget. Its original is saved unchanged. Bounded tiled display is still required before all images up to 50 megapixels can be edited."
            else if (needsColorConsent)
                "The image could not be converted for display. If it has no embedded color profile, you can explicitly treat its colors as sRGB. The original will remain unchanged."
            else "The image could not be displayed safely. Its original remains saved; no smaller replacement was created."
        }
        publish()
    }
    fun assumeSrgb() {
        if (busy || !needsColorConsent) return
        colorAssumed = true; busy = true; exportPreflight = null; checkedExport = null
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
    private fun publish() {
        val doc = document ?: return
        if (camera.viewportWidth <= 1 || camera.viewportHeight <= 1) return
        val savedIds = objects.mapTo(HashSet()) { it.item.objectId }
        val peerById = peerObjects.associateBy { it.item.objectId }
        // Existing objects keep their canonical stacking position. Local
        // transforms and erasures below still override the remote replacement.
        val visibleObjects = objects.map { peerById[it.item.objectId] ?: it }
        val localProvisional = (strokes.mapNotNull { it.drawing } + pendingPreviews.values).filter { it.item.objectId !in savedIds } + listOfNotNull(transientShape)
        val localIds = localProvisional.mapTo(HashSet()) { it.item.objectId }
        val provisional = localProvisional + peerObjects.filter { it.item.objectId !in savedIds && it.item.objectId !in localIds }
        val allTransforms = pendingTransforms.values.fold(emptyMap<String, Transform>()) { a, b -> a + b } + transformed
        val allErased = pendingErases.values.fold(emptySet<String>()) { a, b -> a + b } + erased
        val next = CanvasScene(++sceneRevision, doc, background, camera, core.cameraMatrix(camera), visibleObjects,
            provisional, allErased, allTransforms, selected, hoverPoint, preferences.brush(tool).width, density, preferences.darkTheme,
            connection?.peerViewport?.takeIf { it.documentId == doc.documentId }?.corners.orEmpty())
        scene = next; canvasSink?.invoke(next)
        val viewport = PeerViewport(doc.documentId, core.mapPoints(camera, true, listOf(Point(0.0, 0.0),
            Point(camera.viewportWidth, 0.0), Point(camera.viewportWidth, camera.viewportHeight), Point(0.0, camera.viewportHeight))))
        if (viewport != lastViewport) {
            try { link?.sendViewport(viewport.documentId, viewport.corners); lastViewport = viewport }
            catch (_: SessionFailure) { /* Volatile state is retried on the next view change/reconnect. */ }
        }
    }

    private fun toDocument(sample: PenSample, camera: Camera) = core.mapPoints(camera, true, listOf(Point(sample.x, sample.y))).single()
    private fun selectedLayer(): LayerInfo = document?.layers?.firstOrNull { it.visible && !it.locked && it.blend == "normal" }
        ?: error("There is no unlocked normal annotation layer in this document")

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
                                   val createdAtMs: Long = System.currentTimeMillis())

    override fun beginTool(sample: PenSample, capabilities: StylusCapabilities) {
        cancelTool(); previewJob?.cancel(); cancelNative(previewHandle); previewHandle = null
        if (busy || blockingMessage != null || background == null || strokes.size >= 3) {
            if (strokes.size >= 3) message = "The save queue is full. Wait for the current strokes to finish."
            return
        }
        val current = project ?: return
        this.capabilities = capabilities; hoverPoint = null
        val activeTool = if (sample.tool == PointerTool.Eraser) EditorTool.Eraser else tool
        try {
            val layer = selectedLayer(); val brush = preferences.brush(activeTool); val first = toDocument(sample, camera)
            var run: StrokeRun? = null
            var item: RenderItem? = null
            var anchor: Point? = null
            if (activeTool in listOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker)) {
                val now = System.currentTimeMillis(); val gestureId = core.newId(now.toULong())
                val family = when (activeTool) { EditorTool.Highlighter -> "highlighter"; EditorTool.Marker -> "marker"; else -> "pen" }
                val highlighter = if (family == "highlighter") document?.layers?.filter { it.visible && !it.locked && it.blend == "multiply" }?.minByOrNull { it.id } else null
                val target = if (family == "highlighter") highlighter?.id ?: gestureId else layer.id
                val options = StrokeOptions(gestureId, core.newId(now.toULong()), core.newId(now.toULong()), checkNotNull(document).documentId,
                    layer.id, repository.deviceId, repository.lamport(3), now, family, brush.width, EditorDrawing.rgba(brush.colorArgb),
                    brush.stabilization, brush.pressure.values())
                run = StrokeRun(epoch, camera, sample, first, capabilities, options, target, highlighter?.opacity ?: layer.opacity,
                    if (family == "highlighter") "multiply" else "normal", link)
                invalidateExport(); strokes += run; pending++; startStroke(current, run)
            } else if (activeTool == EditorTool.Select) {
                val snapshot = scene
                val previous = objects.firstOrNull { it.item.objectId == selected }
                if (snapshot != null && previous != null) {
                    val handles = EditorDrawing.screenBounds(snapshot, previous.item)
                    val near = handles.indices.minByOrNull { hypot(handles[it].x - sample.x, handles[it].y - sample.y) }
                    if (near != null && hypot(handles[near].x - sample.x, handles[near].y - sample.y) <= 24 * density) {
                        item = previous.item
                        val r = item.bounds
                        val corners = listOf(Point(r.x, r.y), Point(r.x + r.width, r.y), Point(r.x + r.width, r.y + r.height), Point(r.x, r.y + r.height))
                        anchor = corners[(near + 2) % 4]
                    }
                }
                if (item == null) item = scene?.let { EditorDrawing.hit(it, Point(sample.x, sample.y), 8 * density)?.item }
                selected = item?.objectId
            }
            val newShape = activeTool in listOf(EditorTool.Line, EditorTool.Arrow, EditorTool.Rectangle, EditorTool.Ellipse)
            gesture = ToolGesture(activeTool, camera, first, Point(sample.x, sample.y), layer, brush, run, item, anchor,
                if (item != null || newShape) newId() else null, link, revision = info, newObjectId = if (newShape) newId() else null)
            eraserLast = if (activeTool == EditorTool.Eraser) Point(sample.x, sample.y) else null
            if (activeTool == EditorTool.Text) {
                val hit = scene?.let { EditorDrawing.hit(it, Point(sample.x, sample.y), 8 * density)?.item }
                val text = hit?.shape as? Shape.Text
                textDraft = if (text != null) TextDraft(hit.objectId, text.anchor, text.text, text.font, text.size) else TextDraft(null, first)
            }
            extendTool(listOf(sample)); publish()
        } catch (error: Exception) { cancelTool(); report(error) }
    }

    override fun extendTool(samples: List<PenSample>) {
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
        val active = gesture ?: return
        extendTool(samples)
        if (gesture !== active) return
        gesture = null
        val run = active.stroke
        if (run != null) { run.finished = true; run.prediction = null; run.queue.close(); return }
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

    override fun cancelTool() {
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
    fun predict(sample: PenSample?) { gesture?.stroke?.prediction = sample }

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
        val next = current.document(doc.documentId)
        val drawing = withContext(Dispatchers.Default) { next.render.items.map(EditorDrawing::objectPath) }
        if (token != epoch || project !== current || disposing || !refreshAdmission.accepts(ticket)) return
        // The document and its own revision are one native snapshot. Host sequence
        // alone cannot order optimistic edits sharing the same accepted base.
        if (document?.render?.revision != next.render.revision) clearPeerObjects()
        info = next.render.revision
        document = next; objects = drawing
        if (exportReady?.matches(info, document) != true) exportReady = null
        if (exportPreflight?.revision?.stateHash != info?.stateHash || exportPreflight?.revision?.hostSeq != info?.hostSeq) { exportPreflight = null; checkedExport = null }
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
    fun deleteSelected() { selected?.let { edit(listOf(EditCommand.Delete(it))) }; showContext = false }

    override fun undo() = undoRedo(false)
    override fun redo() = undoRedo(true)
    private fun undoRedo(redo: Boolean) {
        if (busy) return
        cancelTool()
        if (pending != 0) { message = "Wait for the current edit to finish before undo or redo."; return }
        val current = project ?: return
        if (if (redo) info?.canRedo != true else info?.canUndo != true) return
        val options = editOptions(512); val token = epoch; invalidateExport(); pending++
        scope.launch {
            try { mutations.withLock { if (token == epoch) refresh(current, current.undoRedo(options, redo), token) } }
            catch (error: Exception) { report(error) }
            finally { pending-- }
        }
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
        selected = scene?.let { EditorDrawing.hit(it, Point(x, y), 8 * density)?.item?.objectId }
        showContext = true; publish()
    }
    override fun hover(sample: PenSample?, capabilities: StylusCapabilities) {
        this.capabilities = capabilities
        hoverPoint = sample?.let { Point(it.x, it.y) }; publish()
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
            allowDepthReduction = selection.allowDepthReduction, assumeUntaggedSrgb = colorAssumed)
    }
    fun prepareExport() {
        if (busy || pending != 0) return
        val current = project ?: return
        val doc = document ?: return
        val options = try { exportOptions(doc, exportSelection) } catch (error: Exception) { report(error); return }
        busy = true; cancelTool(); exportPreflight = null; checkedExport = null
        val job = scope.launch {
            try {
                transferLabel = "Checking image dimensions, color and export limits…"
                val result = mutations.withLock { repository.preflight(current, options) }
                if (result.revision.stateHash == info?.stateHash && result.revision.hostSeq == info?.hostSeq) { exportPreflight = result; checkedExport = options }
                else message = "The project changed during the check. Check the export again."
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { report(error) }
            finally { busy = false; transferLabel = null; transferJob = null }
        }
        transferJob = job.takeIf { it.isActive }
    }
    fun exportTicket(): ExportTicket? {
        if (pickerTicket != null) { message = "Finish or cancel the pending file destination before starting another export."; return null }
        val current = info ?: return null
        val options = checkedExport ?: return null
        val checked = exportPreflight ?: return null
        if (busy || pending != 0 || checked.revision.stateHash != current.stateHash || checked.revision.hostSeq != current.hostSeq) return null
        return ExportTicket(newId(), current.projectId, options.documentId, current.hostSeq, current.stateHash, options, exportSelection.encoding).also { pickerTicket = it }
    }
    fun cancelExportTicket(id: String?) { if (pickerTicket?.id == id) pickerTicket = null }
    fun export(uri: Uri, ticketId: String?) {
        val ticket = pickerTicket?.takeIf { it.id == ticketId }
        cancelExportTicket(ticketId)
        if (uri.scheme != "content") { message = "The export destination must be a document provider."; return }
        if (disposing || busy || pending != 0 || project == null || document == null || checkedExport == null ||
            ticket?.matches(info, document, checkedExport) != true) {
            message = "The project or export settings changed while choosing a destination. Check the export again."
            scope.launch { try { repository.discardDestination(uri) } catch (error: Exception) { report(error) } }
            return
        }
        exportImage(uri, sharing = false)
    }
    fun sharePng() = exportImage(null, sharing = true)
    private fun exportImage(uri: Uri?, sharing: Boolean) {
        if (busy || pending != 0) return
        val current = project ?: return
        val doc = document ?: return
        val options = checkedExport ?: run { message = "Check the export settings before saving or sharing."; return }
        if (sharing && options.format != ImageFormat.Png8 && options.format != ImageFormat.Png16) {
            message = "Choose PNG 8-bit or PNG 16-bit to share an image."; return
        }
        if (exportPreflight?.revision?.stateHash != info?.stateHash || exportPreflight?.revision?.hostSeq != info?.hostSeq || options.documentId != doc.documentId) {
            exportPreflight = null; checkedExport = null; message = "The project changed. Check the export again."; return
        }
        if (doc.bitDepth > 8u && options.format == ImageFormat.Png8 && !options.allowDepthReduction) {
            message = "Choose PNG 16-bit or explicitly allow reduction to 8-bit."; return
        }
        exportReady = null
        busy = true; cancelTool()
        val job = scope.launch {
            try {
                val result = mutations.withLock {
                    if (sharing) {
                        val shared = repository.sharePng(current, options) { transferLabel = it }
                        shareLease = shared.first; shareUri = shared.first.uri
                        shared.second
                    } else repository.exportImage(checkNotNull(uri), current, options) { transferLabel = it }
                }
                exportReady = ExportReady(result.revision.projectId, doc.documentId, result.revision.stateHash,
                    result.revision.hostSeq, options.region?.width?.toUInt() ?: doc.width,
                    options.region?.height?.toUInt() ?: doc.height, result.blake3)
                message = "${exportSelection.encoding.label} exported at revision ${result.revision.hostSeq}. The original image is unchanged."
            } catch (cancel: CancellationException) {
                message = "Export canceled. The original and saved edits are unchanged."
                throw cancel
            } catch (error: CoreFailure) {
                message = rasterFailure(error, importing = false)
            } catch (error: Exception) { report(error) }
            finally { busy = false; transferLabel = null; transferJob = null }
        }
        transferJob = job.takeIf { it.isActive }
    }
    fun cancelTransfer() {
        if (transferJob != null && transferJob === incomingProducer) explicitImportCancel = true
        transferLabel = "Canceling and removing temporary files…"; transferJob?.cancel()
    }
    private fun rasterFailure(error: CoreFailure, importing: Boolean): String = when (error.kind) {
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
                TransferFailureKind.Unsupported -> "This image or export feature is not supported by the current decoder. The original is unchanged."
                else -> "The image transfer could not finish safely. The original and edits are intact."
            }
            is ImageTooLarge, is RasterTransferFailure -> error.message
            is CoreFailure -> when (error.kind) {
                CoreFailureKind.Backpressure -> "The core is at its resource limit. The unfinished action was canceled; saved work is intact."
                CoreFailureKind.Storage -> "The edit could not be saved. Check available storage and try again."
                CoreFailureKind.Cancelled -> "The unfinished action was canceled."
                else -> "The core could not complete this action (${error.kind}). Saved work is intact."
            }
            is IllegalArgumentException, is IllegalStateException, is UnsupportedOperationException -> error.message ?: "This action could not be completed."
            else -> "This action could not be completed. Saved work is intact."
        }
    }
    override fun onCleared() {
        disposing = true
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
                    try { connectionJob?.cancelAndJoin(); mutations.withLock { closeCurrent() } }
                    finally {
                        try { pairing.close() }
                        finally {
                            shareLease?.let { lease -> cleanupTransfer { MediaHandoff(getApplication()).remove(lease, camera = false) } }
                            shareLease = null; shareUri = null
                        }
                    }
                } catch (error: Exception) { report(error) }
                finally { scope.cancel() }
            }
        }
        super.onCleared()
    }
}

private fun PressureCurve.values() = listOf(0.0, 0.0, x1.toDouble(), y1.toDouble(), x2.toDouble(), y2.toDouble(), 1.0, 1.0)
