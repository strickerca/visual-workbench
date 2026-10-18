package com.visualworkbench.desktop.capture

import com.visualworkbench.bindings.host.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.nio.file.Path

internal data class CapturePickerState(val open: Boolean = false, val busy: Boolean = false,
    val catalog: CapturePickerCatalog? = null, val selected: CaptureSelection? = null,
    val preview: PickerPreview? = null, val region: Boolean = false, val message: String? = null,
    val hotkeyEnabled: Boolean = false, val regionEdges: List<Double>? = null, val regionReady: Boolean = false)

/** UI-dispatcher-owned actions; joins every producer before releasing private
 * files/handles. Selecting a source captures one private preview, never a stream. */
internal class CapturePickerController(
    private val scope: CoroutineScope, val core: WorkbenchCore, private val projects: Path,
    private val device: String, private val openService: suspend () -> CaptureService,
    private val saved: () -> (suspend (WorkbenchProject, Path, String?) -> Unit), private val modal: (Boolean) -> Unit,
    private val live: (CaptureSelection) -> Unit,
) {
    private val mutable = MutableStateFlow(CapturePickerState())
    val state = mutable.asStateFlow()
    val region = CaptureRegionEditor(core::mapPoints)
    fun bindRegion(preview: PickerPreview, camera: Camera) { region.bind(preview.sourceAssetId, preview.sourceWidth, preview.sourceHeight, camera); regionChanged() }
    fun regionBegin(point: Point) { if (!state.value.busy && state.value.region) { region.begin(point); regionChanged() } }
    fun regionMove(point: Point) { if (!state.value.busy) { region.move(point); regionChanged() } }
    fun regionFinish(point: Point) { if (!state.value.busy) { region.finish(point); regionChanged() } }
    fun regionCancel() { region.cancel(); regionChanged() }
    private fun regionChanged() { mutable.update { it.copy(regionEdges = region.edges, regionReady = region.completedEdges() != null) } }
    private val producers = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var service: CaptureService? = null
    private var job: Job? = null
    private var hotkey: RegionPickerHotkey? = null
    private var hotkeyPoll: Job? = null
    private var hotkeyFailed = false
    private var closed = false
    private var generation = 0L
    private var draft: Draft? = null
    private data class Draft(val frame: PickerFrame, val work: PickerWorkspace, val selected: CaptureSelection,
        val destination: suspend (WorkbenchProject, Path, String?) -> Unit)

    fun open(regionOnly: Boolean = false) {
        if (closed) return
        modal(true)
        mutable.update { it.copy(open = true, region = regionOnly && (it.preview == null || it.selected is CaptureSelection.Monitor),
            message = if (regionOnly) "Choose a monitor's Draw region action to select exact snapshot pixels." else null) }
        if (state.value.catalog == null && job == null) refresh()
    }
    fun refresh() = launch {
        discardDraft(); val catalog = hostCall { operation -> service().pickerCatalog(operation) }
        mutable.update { it.copy(catalog = catalog, message = if (catalog.truncated) "The bounded source list is incomplete. Close unused windows and refresh." else null) }
    }
    fun select(source: CaptureSelection, drawRegion: Boolean = false) = launch {
        require(!drawRegion || source is CaptureSelection.Monitor)
        discardDraft(); val ticket = generation; val destination = saved()
        val work = owned(release = { it: PickerWorkspace -> it.retain() }) { PickerWorkspace.create(projects) }
        var frame: PickerFrame? = null; var retained = false
        try {
            val value = hostCall(release = { it: PickerFrame -> it.destroy() }) { op ->
                service().captureSelection(source, id(), 1uL, 1u, work.path.toString(), op)
            }; frame = value
            val pixels = hostCall { op -> value.preview(1024u, op) }
            currentCoroutineContext().ensureActive(); check(!closed && generation == ticket)
            draft = Draft(value, work, source, destination); retained = true
            mutable.update { it.copy(selected = source, preview = pixels, region = drawRegion, message =
                if (drawRegion) "Draw a rectangle on this snapshot. Save keeps these exact pixels and physical coordinates." else "Lossless snapshot ready. Save creates a new capture document.") }
        } finally { if (!retained) withContext(NonCancellable) { try { frame?.destroy() } finally { owned { work.retain() } } } }
    }
    fun save() = launch {
        val current = checkNotNull(draft); var chosen = current
        var cropped: Draft? = null; var opened: WorkbenchProject? = null; var published: Path? = null
        var warning: String? = null
        try {
            if (state.value.region) {
                val edges = checkNotNull(region.completedEdges()) { "Draw and finish a nonempty rectangle first" }
                val work = owned(release = { it: PickerWorkspace -> it.retain() }) { PickerWorkspace.create(projects) }
                var frame: PickerFrame? = null
                try {
                    val value = hostCall(release = { it: PickerFrame -> it.destroy() }) { op -> current.frame.cropRegion(edges, work.path.toString(), op) }
                    frame = value; chosen = Draft(value, work, current.selected, current.destination); cropped = chosen
                } catch (error: Throwable) { withContext(NonCancellable) { try { frame?.destroy() } finally { owned { work.retain() } } }; throw error }
            }
            val info = chosen.frame.info(); val f = info.frame
            val projectId = id(); val documentId = id(); val destination = projects.resolve("$projectId.vwb")
            val project = createCaptureProject(CreateFileProject(destination.toString(), projectId, documentId, id(), device,
                "Screen capture", chosen.work.path.resolve("capture.png").toString(), chosen.work.path.toString(), System.currentTimeMillis()),
                CaptureImportDescriptor(true, f.captureSessionId, f.frameId, f.geometryRevision, "windows", info.sourceKind,
                    f.physicalX, f.physicalY, f.width, f.height, f.dpiScale, f.timestampNs, f.capturedAtMs,
                    f.windowHandle, info.monitorId, f.sourceAssetId))
            opened = project; published = destination
            if (info.semanticWindowAvailable) {
                val revision = project.info()
                val ticket = semanticWorkflows(project).beginCapture(WorkflowBinding(revision.projectId, documentId, revision.hostSeq, revision.stateHash))
                try {
                    val tree = hostCall { op -> chosen.frame.collect(op) }
                    require(tree.sourceAssetId == ticket.describe().sourceAssetId)
                    val plan = ticket.prepare(WorkflowMetadata(id(), device, revision.nextLamport, System.currentTimeMillis()), id(),
                        SemanticPlatform.Uia, tree.frameDeltaMs, tree.collectionElapsedMs, SemanticBoundsSpace.HostPhysical,
                        tree.elements.map { e -> CapturedSemanticElement(e.localId, e.parentLocalId, e.name, e.role, e.automationId,
                            e.resourceId, e.htmlId, Rect(e.x, e.y, e.width, e.height), e.text, e.enabled, e.focused) })
                    try { plan.commit() } finally { plan.close() }
                } catch (error: CancellationException) { throw error }
                catch (_: Exception) { warning = "Capture saved; the selected window's semantic snapshot was unavailable or stale." }
                finally { ticket.close() }
            } else warning = "Capture saved. Monitor and region captures have no selected window subtree; no semantic snapshot was synthesized."
            current.destination(project, destination, warning); opened = null
            discardDraft(); cropped?.let { release(it); cropped = null }
            mutable.update { it.copy(open = false, message = warning) }; modal(false)
        } finally {
            withContext(NonCancellable) {
                try { opened?.close() }
                finally { cropped?.let { value -> try { value.frame.destroy() } finally { owned { if (published != null) value.work.release() else value.work.retain() } } } }
            }
            if (published != null && currentCoroutineContext().isActive.not()) {
                mutable.update { it.copy(message = "A capture project was saved before cancellation. Its original remains in the project folder.") }
            }
        }
    }
    fun startLive() = launch {
        val selected = checkNotNull(draft).selected
        val source = if (state.value.region) {
            val monitor = selected as? CaptureSelection.Monitor ?: error("Choose a monitor for a live region")
            pickerRegion(monitor.target, checkNotNull(region.completedEdges()), 1u)
        } else selected
        // Live always needs a fresh explicit local action, including reconnect.
        // The native producer rechecks this exact geometry; no window fallback.
        live(source)
        discardDraft(); mutable.update { it.copy(open = false) }; modal(false)
    }
    fun cancelWork() { generation++; regionCancel(); job?.cancel() }
    suspend fun dismiss() {
        generation++; job?.cancelAndJoin()
        try { discardDraft() }
        catch (_: Exception) { mutable.update { it.copy(message = "Private capture cleanup was refused. The original remains in the picker inbox.") } }
        finally { mutable.update { it.copy(open = false, busy = false) }; modal(false) }
    }
    fun enableRegionHotkey(modifiers: UInt = 3u, key: UInt = 82u) = launch {
        check(!hotkeyFailed); releaseHotkey()
        val value = owned(release = { it: RegionPickerHotkey -> try { it.shutdown() } finally { it.destroy() } }) { registerRegionPickerHotkey(modifiers, key) }
        hotkey = value; mutable.update { it.copy(hotkeyEnabled = true) }
        hotkeyPoll = scope.launch { try { while (isActive) { if (value.take()) open(true); delay(15) } }
            catch (error: CancellationException) { throw error }
            catch (_: Exception) { disableRegionHotkey(); mutable.update { it.copy(message = "Region hotkey stopped. Enable it again to retry.") } } }
    }
    fun disableRegionHotkey() {
        try { releaseHotkey() }
        catch (_: Exception) { hotkeyFailed = true; mutable.update { it.copy(message = "Region hotkey cleanup could not be confirmed. Restart the application before enabling it again.") } }
    }
    private fun releaseHotkey() {
        hotkeyPoll?.cancel(); hotkeyPoll = null
        val value = hotkey; hotkey = null
        try { value?.let { try { it.shutdown() } finally { it.destroy() } } }
        finally { mutable.update { it.copy(hotkeyEnabled = false) } }
    }
    suspend fun shutdown() {
        if (closed) return; closed = true; generation++
        withContext(NonCancellable) {
            try { releaseHotkey() }
            finally { try { job?.cancelAndJoin() }
                finally { val value = draft; draft = null
                    try { try { value?.frame?.destroy() } finally { value?.let { owned { it.work.retain() } } } }
                    finally { try { service?.destroy() } finally { service = null; producers.cancel(); modal(false) } }
                }
            }
        }
    }
    private suspend fun service(): CaptureService = service ?: openService().also { service = it }
    private fun id(): String = core.newId(System.currentTimeMillis().toULong())
    private suspend fun release(value: Draft) { withContext(NonCancellable) { try { value.frame.destroy() } finally { owned { value.work.release() } } } }
    private suspend fun discardDraft() { generation++; region.detach(); val value = draft; draft = null
        mutable.update { it.copy(preview = null, selected = null, regionEdges = null, regionReady = false) }; if (value != null) release(value) }
    private fun launch(body: suspend () -> Unit) {
        if (closed || job != null) return
        mutable.update { it.copy(busy = true, message = null) }
        val next = scope.launch(start = CoroutineStart.LAZY) {
            try { body() }
            catch (error: CancellationException) { throw error }
            catch (_: Exception) { mutable.update { it.copy(message = "Capture was refused or did not finish. The source may have moved, lack capture exclusion, or exceed the memory limit. Any published original is preserved.") } }
        }
        job = next
        next.invokeOnCompletion { scope.launch(NonCancellable) { if (job === next) { job = null; mutable.update { it.copy(busy = false) } } } }
        next.start()
    }
    private suspend fun <T> hostCall(release: suspend (T) -> Unit = {}, body: suspend (CaptureOperation) -> T): T {
        currentCoroutineContext().ensureActive(); val operation = CaptureOperation()
        try { return owned(release, { operation.cancel() }) { body(operation) } }
        finally { operation.destroy() }
    }
    private suspend fun <T> owned(release: suspend (T) -> Unit = {}, signal: () -> Unit = {}, body: suspend () -> T): T =
        settledPicker(producers, release, signal, body)
}
