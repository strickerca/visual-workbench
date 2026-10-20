package com.visualworkbench.desktop.capture

import com.visualworkbench.bindings.host.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.nio.file.Path
import java.util.concurrent.atomic.AtomicReference

internal data class CapturePickerState(
    val open: Boolean = false, val busy: Boolean = false,
    val catalog: CapturePickerCatalog? = null, val selected: CaptureSelection? = null,
    val preview: PickerPreview? = null, val previewReady: Boolean = false, val region: Boolean = false,
    val regionOnly: Boolean = false, val message: String? = null,
    val hotkeyEnabled: Boolean = false, val regionEdges: List<Double>? = null,
    val regionReady: Boolean = false, val cleanupPending: Boolean = false, val stopping: Boolean = false,
    val canShareLive:Boolean=false,
)

/** Object identity fences a queued action even if two snapshots hash alike. */
internal fun CapturePickerState.acceptsPreview(value: PickerPreview?): Boolean =
    open && previewReady && preview != null && preview === value && !cleanupPending && !stopping

/** UI-dispatcher owner of local snapshots only. The destination is bound before
 * capture; its callback takes project ownership at invocation, including failure.
 * A failed retirement keeps this owner and its modal lease alive for retry. */
internal class CapturePickerController(
    private val scope: CoroutineScope, val core: WorkbenchCore, private val projects: Path,
    private val device: String, private val openService: suspend () -> CaptureService,
    private val saved: () -> (suspend (WorkbenchProject, Path, String?) -> Unit),
    private val modal: (Boolean) -> Unit,
    private val mayOpen: () -> Boolean = { true },
    private val delivered: (suspend (CaptureTarget, CaptureFrameInfo, ProjectInfo, String) -> Unit)? = null,
    private val liveDestination: (() -> (suspend (CaptureSelection,String)->Unit)?)? = null,
) {
    private val mutable = MutableStateFlow(CapturePickerState())
    val state = mutable.asStateFlow()
    private val operations = PickerOperationOwner(scope) { busy -> mutable.update { it.copy(busy = busy) } }
    private val producers = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var service: CaptureService? = null
    private val serviceRelease = PickerReleaseAction { service?.destroy() }
    private var hotkey: RegionPickerHotkey? = null
    private var hotkeyRelease: PickerReleaseAction? = null
    private var hotkeyPoll: Job? = null
    private var hotkeyFailed = false
    private var stopped = false
    private val drafts = mutableListOf<Draft>()
    private var snapshot: Draft? = null
    private var pendingProject: WorkbenchProject? = null
    private var pendingProjectFailure: Throwable? = null
    private class Draft(
        val work: PickerWorkspace, val selected: CaptureSelection,
        val destination: suspend (WorkbenchProject, Path, String?) -> Unit,
        val live:(suspend (CaptureSelection,String)->Unit)?,
    ) {
        val frame = AtomicReference<PickerFrame?>(null)
        val destroy = PickerReleaseAction { frame.get()?.destroy() }
    }

    val region = CaptureRegionEditor(core::mapPoints)
    private var regionCamera: Camera? = null
    fun bindRegion(preview: PickerPreview, camera: Camera) {
        // A delayed composition of an old preview cannot rebind the next draft.
        if (preview !== state.value.preview || state.value.cleanupPending || !operations.accepting) return
        region.bind(preview.sourceAssetId, preview.sourceWidth, preview.sourceHeight, camera)
        regionCamera = camera
        regionChanged()
    }
    fun previewPresented(preview: PickerPreview) {
        if (preview !== state.value.preview || state.value.cleanupPending || !operations.accepting) return
        mutable.update { it.copy(previewReady = true) }
    }
    fun previewUnavailable(preview: PickerPreview) {
        if (preview !== state.value.preview) return
        regionCancel()
        mutable.update { it.copy(preview = null, previewReady = false, message = "The preview could not be displayed. Refresh sources and select again before saving.") }
    }
    fun regionBegin(point: Point, preview: PickerPreview, camera: Camera) { if (regionAllowed(preview, camera)) { region.begin(point); regionChanged() } }
    fun regionMove(point: Point, preview: PickerPreview, camera: Camera) { if (regionAllowed(preview, camera)) { region.move(point); regionChanged() } }
    fun regionFinish(point: Point, preview: PickerPreview, camera: Camera) { if (regionAllowed(preview, camera)) { region.finish(point); regionChanged() } }
    fun regionCancel(preview: PickerPreview? = null, camera: Camera? = null) {
        if ((preview == null || preview === state.value.preview) && (camera == null || camera === regionCamera)) {
            region.cancel(); regionChanged()
        }
    }
    private fun regionAllowed(preview: PickerPreview, camera: Camera): Boolean =
        state.value.acceptsPreview(preview) && camera === regionCamera && operations.accepting && state.value.open &&
        !state.value.busy && !state.value.cleanupPending && state.value.region
    private fun regionChanged() { mutable.update { it.copy(regionEdges = region.edges, regionReady = region.completedEdges() != null) } }

    fun open(regionOnly: Boolean = false) {
        if (!operations.accepting || state.value.busy || state.value.cleanupPending || !mayOpen()) return
        try { modal(true) } // Seal focus admission before the window becomes observable.
        catch (error: CancellationException) { throw error }
        catch (error: Exception) { cleanupRefused(); return }
        if (regionOnly) regionCancel()
        mutable.update { old ->
            val monitor = old.selected is CaptureSelection.Monitor
            old.copy(open = true, regionOnly = regionOnly,
                region = if (regionOnly) monitor else old.region,
                preview = if (regionOnly && !monitor) null else old.preview,
                previewReady = old.previewReady && (!regionOnly || monitor),
                message = if (regionOnly) "Choose a monitor's Draw region action to select exact snapshot pixels." else old.message)
        }
        if (state.value.catalog == null) refresh()
    }

    fun refresh() = launch { ticket ->
        discardDrafts(preserve = false)
        val catalog = hostCall { operation -> service().pickerCatalog(operation) }
        requireCurrent(ticket)
        mutable.update { it.copy(catalog = catalog, message = if (catalog.truncated)
            "The bounded source list is incomplete. Close unused windows and refresh." else null) }
    }

    fun select(source: CaptureSelection, drawRegion: Boolean = false) = launch { ticket ->
        require(!drawRegion || source is CaptureSelection.Monitor)
        require(!state.value.regionOnly || drawRegion)
        val catalog = checkNotNull(state.value.catalog)
        require(when (source) {
            is CaptureSelection.Window -> catalog.windows.any { it.target == source.target }
            is CaptureSelection.Monitor -> source.target in catalog.monitors
            is CaptureSelection.Region -> false
        }) // Never adopt an ambient or stale catalog target supplied by a caller.
        discardDrafts(preserve = false)
        val current = newDraft(source, saved())
        var ready = false
        try {
            val value = hostCall(release = { current.frame.set(it) }) { operation ->
                guardedMirrorCapture(source.tunnelSource(),{current.frame.set(it)}) {
                    service().captureSelection(source, id(), 1uL, 1u, current.work.path.toString(), operation)
                }
            }
            current.frame.set(value)
            val pixels = hostCall { operation -> value.preview(1024u, operation) }
            val receipt = value.info().frame
            require(pixels.sourceAssetId == receipt.sourceAssetId &&
                pixels.sourceWidth == receipt.width && pixels.sourceHeight == receipt.height)
            validatePickerPreview(pixels)
            requireCurrent(ticket)
            snapshot = current
            mutable.update { it.copy(selected = source, preview = pixels, previewReady = false, region = drawRegion,
                canShareLive=current.live!=null,
                message = if (drawRegion) "Draw a rectangle on this snapshot. Save keeps these exact pixels and physical coordinates."
                else "Lossless snapshot ready. Save creates a new capture document.") }
            ready = true
        } finally {
            if (!ready) withContext(NonCancellable) { discardDrafts(preserve = true) }
        }
    }

    fun save(preview: PickerPreview?) {
        if (state.value.busy || !state.value.acceptsPreview(preview)) return
        launch { ticket ->
            val current = checkNotNull(snapshot)
            check(state.value.acceptsPreview(preview))
            var chosen = current
            var cropped: Draft? = null
            var published = false
            var warning: String? = null
            try {
                if (state.value.region) {
                    val edges = checkNotNull(region.completedEdges())
                    val crop = newDraft(current.selected, current.destination)
                    cropped = crop
                    val value = hostCall(release = { crop.frame.set(it) }) { operation ->
                        checkNotNull(current.frame.get()).cropRegion(edges, crop.work.path.toString(), operation)
                    }
                    crop.frame.set(value)
                    chosen = crop
                }
                requireCurrent(ticket)
                val frame = checkNotNull(chosen.frame.get())
                val info = frame.info(); val f = info.frame
                val projectId = id(); val documentId = id(); val destination = projects.resolve("$projectId.vwb")
                val project = createCaptureProject(CreateFileProject(destination.toString(), projectId, documentId, id(), device,
                    "Screen capture", chosen.work.path.resolve("capture.png").toString(), chosen.work.path.toString(), System.currentTimeMillis()),
                    CaptureImportDescriptor(true, f.captureSessionId, f.frameId, f.geometryRevision, "windows", info.sourceKind,
                        f.physicalX, f.physicalY, f.width, f.height, f.dpiScale, f.timestampNs, f.capturedAtMs,
                        f.windowHandle, info.monitorId, f.sourceAssetId))
                pendingProject = project; published = true
                requireCurrent(ticket)
                if (info.semanticWindowAvailable) {
                    val revision = project.info()
                    val workflow = semanticWorkflows(project).beginCapture(WorkflowBinding(revision.projectId, documentId, revision.hostSeq, revision.stateHash))
                    try {
                        val tree = hostCall { operation -> frame.collect(operation) }
                        requireCurrent(ticket)
                        require(tree.sourceAssetId == workflow.describe().sourceAssetId)
                        val plan = workflow.prepare(WorkflowMetadata(id(), device, revision.nextLamport, System.currentTimeMillis()), id(),
                            if (tree.platform == "chromium_uia") SemanticPlatform.ChromiumUia else SemanticPlatform.Uia,
                            tree.frameDeltaMs, tree.collectionElapsedMs, SemanticBoundsSpace.HostPhysical,
                            tree.elements.map { e -> CapturedSemanticElement(e.localId, e.parentLocalId, e.name, e.role, e.automationId,
                                e.resourceId, e.htmlId, Rect(e.x, e.y, e.width, e.height), e.text, e.enabled, e.focused) })
                        try { plan.commit() } finally { plan.close() }
                    } catch (error: CancellationException) { throw error }
                    catch (_: Exception) { warning = "Capture saved; the selected window's semantic snapshot was unavailable or stale." }
                    finally { workflow.close() }
                } else warning = "Capture saved. Monitor and region captures have no selected window subtree."
                requireCurrent(ticket)
                // The callback owns close even if route validation or delivery throws.
                pendingProject = null
                // A monitor or cropped region is not a full selected-window
                // origin. Never invent a window selector for those captures.
                val window = (chosen.selected as? CaptureSelection.Window)?.target
                    ?.takeIf { !state.value.region && info.sourceKind == "window" }
                deliverCaptureProject(project, { current.destination(it, destination, warning) },
                    observation = if (delivered == null || window == null) null else { revision -> delivered.invoke(window, f, revision, documentId) },
                    observationUnavailable = {
                        warning = listOfNotNull(warning, "Capture saved; verification capture identity was unavailable. Choose a fresh capture target before verification.").joinToString(" ")
                    },
                ) { held, error ->
                    pendingProject = held
                    pendingProjectFailure = error
                    cleanupRefused()
                }
                discardDrafts(preserve = false)
                try { modal(false) } catch (error: Exception) { cleanupRefused(); throw error }
                mutable.update { it.copy(open = false, message = warning) }
            } finally {
                withContext(NonCancellable) {
                    closePendingProject()
                    cropped?.takeIf { it in drafts }?.let { releaseDraft(it, preserve = !published) }
                }
                if (!currentCoroutineContext().isActive) mutable.update { it.copy(message =
                    "Capture cancelled. Any published project and completed original remain saved; inspect them before retrying.") }
            }
        }
    }

    fun cancelWork() { operations.invalidate(); regionCancel() }

    /** A second explicit action approves continuous capture of this exact
     * preview selection. No peer request can invoke this local approval. */
    fun shareLive(preview:PickerPreview?) {
        if(state.value.busy||!state.value.acceptsPreview(preview)||!state.value.canShareLive)return
        launch { ticket->
            val current=checkNotNull(snapshot)
            check(state.value.acceptsPreview(preview))
            var cropped:Draft?=null
            try {
                val selected=if(state.value.region){
                    val target=(current.selected as? CaptureSelection.Monitor)?.target?:error("Region monitor missing")
                    val crop=newDraft(current.selected,current.destination);cropped=crop
                    val value=hostCall(release={crop.frame.set(it)}){operation->checkNotNull(current.frame.get()).cropRegion(checkNotNull(region.completedEdges()),crop.work.path.toString(),operation)}
                    crop.frame.set(value)
                    val frame=value.info().frame
                    CaptureSelection.Region(target,frame.physicalX,frame.physicalY,frame.width,frame.height)
                }else current.selected
                requireCurrent(ticket)
                val name=when(selected){is CaptureSelection.Window->state.value.catalog?.windows?.firstOrNull{it.target==selected.target}?.title?:"Selected window";is CaptureSelection.Monitor->"Selected monitor";is CaptureSelection.Region->"Selected region"}
                checkNotNull(current.live)(selected,name)
                requireCurrent(ticket)
                discardDrafts(preserve=false)
                modal(false)
                mutable.update{it.copy(open=false,canShareLive=false,message=null)}
            }finally {withContext(NonCancellable){cropped?.takeIf{it in drafts}?.let{releaseDraft(it,preserve=false)}}}
        }
    }

    /** Returns only after all capture work and private cleanup has settled.
     * Refusal throws while retaining the modal lease and exact remaining owner. */
    suspend fun dismiss() {
        if (operations.stopping) { shutdown(); return }
        operations.retire {
            try {
                if (hotkeyFailed) releaseHotkey()
                discardDrafts(preserve = false)
                modal(false)
                mutable.update { it.copy(open = false, cleanupPending = false) }
            } catch (error: Exception) { cleanupRefused(); throw error }
        }
    }

    fun enableRegionHotkey(modifiers: UInt = 3u, key: UInt = 82u) = launch { ticket ->
        check(!hotkeyFailed)
        releaseHotkey()
        var installed = false
        try {
            // A late-created registration remains owned through retirement.
            val value = owned(release = ::ownHotkey) { registerRegionPickerHotkey(modifiers, key) }
            ownHotkey(value)
            requireCurrent(ticket)
            mutable.update { it.copy(hotkeyEnabled = true) }
            hotkeyPoll = scope.launch {
                try { while (isActive) { if (value.take()) open(true); delay(15) } }
                catch (error: CancellationException) { throw error }
                catch (_: Exception) {
                    mutable.update { it.copy(message = "Region hotkey stopped. Disable it before retrying.") }
                }
            }
            installed = true
        } finally { if (!installed) withContext(NonCancellable) { releaseHotkey() } }
    }

    fun disableRegionHotkey() = launch { releaseHotkey() }
    private fun ownHotkey(value: RegionPickerHotkey) {
        hotkey = value
        hotkeyRelease = PickerReleaseAction { value.shutdown(); value.destroy() }
    }
    private suspend fun releaseHotkey() = withContext(NonCancellable) {
        hotkeyPoll?.cancelAndJoin(); hotkeyPoll = null
        if (hotkey == null) return@withContext
        val release = checkNotNull(hotkeyRelease)
        try {
            owned { release.close() }
            hotkey = null
            hotkeyRelease = null
            hotkeyFailed = false
            mutable.update { it.copy(hotkeyEnabled = false) }
        } catch (error: Exception) {
            hotkeyFailed = true
            cleanupRefused()
            throw error
        }
    }

    suspend fun shutdown() {
        if (stopped) return
        mutable.update { it.copy(stopping = true, previewReady = false) }
        operations.retire(permanent = true) {
            try {
                releaseHotkey()
                discardDrafts(preserve = true)
                owned { serviceRelease.close() }; service = null
                modal(false)
                producers.cancel()
                mutable.update { it.copy(open = false, cleanupPending = false) }
                stopped = true
            } catch (error: Exception) { cleanupRefused(); throw error }
        }
    }

    private suspend fun newDraft(source: CaptureSelection, destination: suspend (WorkbenchProject, Path, String?) -> Unit): Draft =
        withContext(NonCancellable) {
            // Bounded filesystem setup is installed in its caller owner before
            // cancellation may resume. No capture starts until this owner exists.
            val work = withContext(Dispatchers.IO) { PickerWorkspace.create(projects) }
            Draft(work, source, destination,liveDestination?.invoke()).also { drafts += it }
        }
    private suspend fun service(): CaptureService = service ?: openService().also { service = it }
    private fun id(): String = core.newId(System.currentTimeMillis().toULong())
    private suspend fun requireCurrent(ticket: Long) {
        currentCoroutineContext().ensureActive()
        if (!operations.current(ticket)) throw CancellationException("Picker generation retired")
    }
    private suspend fun releaseDraft(value: Draft, preserve: Boolean) = withContext(NonCancellable) {
        try {
            owned { value.destroy.close() }; value.frame.set(null)
            owned { if (preserve) value.work.retain() else value.work.release() }
            drafts.remove(value)
            if (snapshot === value) snapshot = null
        } catch (error: Exception) { cleanupRefused(); throw error }
    }
    private suspend fun closePendingProject() {
        pendingProjectFailure?.let { throw it }
        try { pendingProject?.let { it.close(); pendingProject = null } }
        catch (error: Exception) { pendingProjectFailure = error; cleanupRefused(); throw error }
    }
    private suspend fun discardDrafts(preserve: Boolean) {
        closePendingProject()
        regionCamera = null
        region.detach()
        mutable.update { it.copy(preview = null, previewReady = false, selected = null, regionEdges = null, regionReady = false) }
        for (value in drafts.toList()) releaseDraft(value, preserve)
        mutable.update { it.copy(cleanupPending = false) }
    }
    private fun cleanupRefused() {
        mutable.update { it.copy(cleanupPending = true, preview = null, previewReady = false, regionReady = false,
            message = "Capture cleanup is pending. Keep this window open and retry closing before leaving the editor.") }
    }
    private fun launch(body: suspend (Long) -> Unit) {
        if (!state.value.open || state.value.cleanupPending) return
        operations.start { ticket ->
            mutable.update { it.copy(message = null) }
            try { body(ticket) }
            catch (error: CancellationException) { throw error }
            catch (_: Exception) {
                if (!state.value.cleanupPending) mutable.update { it.copy(message =
                    "Capture was refused or did not finish. Refresh the source list and retry. Any published original is preserved.") }
            }
        }
    }
    private suspend fun <T> hostCall(release: suspend (T) -> Unit = {}, body: suspend (CaptureOperation) -> T): T {
        currentCoroutineContext().ensureActive()
        val operation = CaptureOperation()
        try { return owned(release, { operation.cancel() }) { body(operation) } }
        finally { operation.destroy() }
    }
    private suspend fun <T> owned(release: suspend (T) -> Unit = {}, signal: () -> Unit = {}, body: suspend () -> T): T =
        settledPicker(producers, release, signal, body)
}
