package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

public enum class EditorWorkflowPanel { None, Documents, SavedViews, Layers, Tones, OfflineReview }
public data class EditorWorkflowAttachment(public val project: WorkbenchProject,
    public val document: DocumentSnapshot, public val epoch: Long, public val sequence: ULong,
    public val deviceId: String)

/** UI-confined composition of borrowed workflow leaves. The existing editor
 * retains project, rendering, input and document adoption. Its mutation mutex is
 * also the gate for navigation and review; callbacks must not reacquire it. */
public class EditorWorkflows(
    private val scope: CoroutineScope,
    private val mutations: Mutex,
    private val attachment: () -> EditorWorkflowAttachment?,
    private val idle: () -> Boolean,
    private val cancelInput: () -> Unit,
    private val metadata: suspend (WorkbenchProject) -> WorkflowMetadata,
    private val newId: () -> String,
    private val camera: () -> Camera,
    private val applyCamera: (Camera) -> Unit,
    private val updated: suspend (EditorWorkflowAttachment, ProjectInfo) -> Unit,
    private val committed: suspend (EditorWorkflowAttachment, WorkflowReceipt) -> Unit,
    private val adoptDocument: suspend (EditorWorkflowAttachment, DocumentSnapshot) -> Unit,
    private val report: (String) -> Unit,
    localViews: (WorkbenchProject) -> WorkbenchLocalViews,
    documentControls: (WorkbenchProject) -> WorkbenchDocumentControls,
    reviews: (WorkbenchProject) -> WorkbenchOfflineReview,
    toneControls: ((WorkbenchProject) -> WorkbenchToneControls)? = null,
) {
    private val mutable: MutableStateFlow<EditorWorkflowPanel> = MutableStateFlow(EditorWorkflowPanel.None)
    public val panel: StateFlow<EditorWorkflowPanel> = mutable
    public val navigator: DocumentNavigator = DocumentNavigator()
    public val views: LocalViewsInteraction = LocalViewsInteraction(scope, newId, localViews)
    private var navigation: Job? = null
    private var observed: OfflineReviewBinding? = null
    private var departing: Boolean = false
    private var closed: Boolean = false
    private data class ViewOwner(val project: WorkbenchProject, val documentId: String, val epoch: Long)
    private var viewOwner: ViewOwner? = null
    private var reviewDetached: Boolean = false
    public val controls: DocumentControlsInteraction = DocumentControlsInteraction(scope, mutations, metadata,
        { admits(EditorWorkflowPanel.Layers) }, { receipt ->
            val a = attachment()
            if (a != null && receipt.revision.projectId == a.document.render.revision.projectId) committed(a, receipt)
        }, documentControls)
    public val tones: ToneControlsInteraction = ToneControlsInteraction(scope, mutations, newId, metadata,
        { admits(EditorWorkflowPanel.Tones) }, { receipt ->
            val a=attachment()
            if(a!=null&&receipt.revision.projectId==a.document.render.revision.projectId)committed(a,receipt)
        }, { project -> toneControls?.invoke(project) ?: throw WorkflowFailure(WorkflowFailureKind.Invalid) })
    public val review: OfflineReviewInteraction = OfflineReviewInteraction(scope, mutations,
        { attachment()?.takeUnless { departing }?.let { OfflineReviewAttachment(it.project, it.document.render.revision, it.epoch) } },
        { admits(EditorWorkflowPanel.OfflineReview) }, metadata, newId,
        { a, info -> current(a)?.let { updated(it, info) } },
        { a, receipt -> current(a)?.let { committed(it, receipt) } }, reviews)

    /** Used by ordinary input/actions, including shortcuts and peer-follow. */
    public fun blocksInput(): Boolean = closed || departing || panel.value != EditorWorkflowPanel.None ||
        navigation != null || controls.state.value.busy || controls.state.value.retry != null || tones.state.value.blocksInput || (review.state.value.busy && !reviewDetached)
    private fun visible(a: EditorWorkflowAttachment): Boolean = observed?.let { it == a.document.render.revision.offlineReviewBinding() } != false
    private fun admits(owner: EditorWorkflowPanel): Boolean {
        val a = attachment() ?: return false
        return !departing && panel.value == owner && navigation == null && idle() && visible(a) &&
            (owner == EditorWorkflowPanel.Layers || controls.state.value.mayLeave) &&
            (owner == EditorWorkflowPanel.Tones || tones.state.value.mayLeave) &&
            (owner == EditorWorkflowPanel.OfflineReview || !review.state.value.busy)
    }
    private fun current(a: OfflineReviewAttachment): EditorWorkflowAttachment? = attachment()?.takeIf {
        !departing && it.project === a.project && it.epoch == a.epoch
    }
    public fun bind(a: EditorWorkflowAttachment): Unit {
        if (closed || departing || attachment()?.let { it.project === a.project && it.epoch == a.epoch && it.document === a.document } != true) return
        if (reviewDetached) { review.resume(); reviewDetached = false }
        navigator.bind(a.document, a.epoch)
        controls.bind(a.project, a.document, a.epoch, a.sequence)
        tones.bind(a.project, a.document, a.epoch, a.sequence)
        if (visible(a)) observed = null
        if (panel.value == EditorWorkflowPanel.OfflineReview) review.refresh()
    }
    /** Revoke stale panel events before the root's asynchronous snapshot read. */
    public fun observe(info: ProjectInfo, sequence: ULong): Unit {
        observed = info.offlineReviewBinding(); navigator.invalidate(); controls.observe(sequence); tones.observe(sequence)
    }
    public fun activate(value: EditorWorkflowPanel, active: Boolean): Unit {
        if (!active) {
            if (panel.value == value && value==EditorWorkflowPanel.Tones && !tones.active(false)) {
                report("Apply or explicitly discard the private tone changes before closing."); return
            }
            if (panel.value == value) { mutable.value = EditorWorkflowPanel.None; navigator.active(false); views.active(false); controls.active(false) }
            return
        }
        val a = attachment() ?: return
        if (closed || departing || !idle() || navigation != null || !visible(a) ||
            (panel.value != EditorWorkflowPanel.None && panel.value != value) ||
            (value != EditorWorkflowPanel.Layers && !controls.state.value.mayLeave) ||
            (value != EditorWorkflowPanel.Tones && !tones.state.value.mayLeave) || review.state.value.busy) {
            report("Finish the current edit or dialog before opening these controls."); return
        }
        if (a.document.page != null && (value == EditorWorkflowPanel.SavedViews || value == EditorWorkflowPanel.Layers || value == EditorWorkflowPanel.Tones)) {
            report("Saved views and layer controls currently apply to image documents."); return
        }
        if(value==EditorWorkflowPanel.Tones&&giantDisplayRequired(a.document)) {
            report("Tone previews currently require a standard image document. The original is preserved.");return
        }
        cancelInput(); mutable.value = value
        navigator.active(value == EditorWorkflowPanel.Documents); views.active(value == EditorWorkflowPanel.SavedViews)
        controls.active(value == EditorWorkflowPanel.Layers)
        tones.active(value == EditorWorkflowPanel.Tones)
        if (value == EditorWorkflowPanel.OfflineReview) review.refresh()
    }
    public fun mayLeave(): Boolean {
        if (!tones.state.value.mayLeave) { report("Apply or explicitly discard the private tone changes before leaving this document."); return false }
        if (!controls.state.value.mayLeave) { report("Finish the layer edit or explicitly discard its retry handle before leaving this document."); return false }
        if (navigation != null) { report("Finish or cancel the document switch first."); return false }
        return true
    }
    /** Restore a device-local image camera after the root admits/fits a document. */
    public suspend fun enter(a: EditorWorkflowAttachment): Unit {
        if(closed||departing)return
        val owner = ViewOwner(a.project, a.document.documentId, a.epoch)
        if (viewOwner?.let { it.project === owner.project && it.documentId == owner.documentId && it.epoch == owner.epoch } == true) return
        viewOwner = null
        if (a.document.page != null) { views.detach(); return }
        val restored = views.attach(a.project, LocalViewsBinding(a.document.render.revision.projectId,
            a.document.documentId, a.deviceId), a.epoch, camera())
        val now = attachment()
        if (now == null || now.project !== a.project || now.epoch != a.epoch || now.document.documentId != a.document.documentId || departing) {
            views.detach(); return
        }
        viewOwner = owner
        restored?.let(applyCamera)
        views.remember(camera(), a.epoch, a.document.documentId)
    }
    public fun remember(value: Camera): Unit {
        val a = attachment() ?: return
        if (!departing && a.document.page == null) views.remember(value, a.epoch, a.document.documentId)
    }
    public fun saveView(binding: LocalViewsBinding, epoch: Long, name: String): Unit {
        if (panel.value == EditorWorkflowPanel.SavedViews && idle()) views.addBookmark(binding, epoch, name, camera())
    }
    public fun restoreView(binding: LocalViewsBinding, epoch: Long, id: String): Unit {
        val a = attachment() ?: return
        if (panel.value != EditorWorkflowPanel.SavedViews || !idle() || a.epoch != epoch ||
            a.document.documentId != binding.documentId || a.document.render.revision.projectId != binding.projectId || a.deviceId != binding.deviceId) return
        views.restoreBookmark(binding, id, camera(), epoch)?.let { cancelInput(); applyCamera(it) }
    }
    public fun choose(request: DocumentNavigationRequest): Unit {
        val a = attachment()
        if (a == null || closed || departing || navigation != null || panel.value != EditorWorkflowPanel.Documents ||
            !idle() || !controls.state.value.mayLeave || !tones.state.value.mayLeave || !navigator.matches(request, a.document, a.epoch)) {
            navigator.finish(request); return
        }
        cancelInput()
        lateinit var work: Job
        work = scope.launch(start = CoroutineStart.LAZY) {
            try {
                mutations.withLock {
                    fun valid(): Boolean = !departing && idle() && attachment()?.let {
                        it.project === a.project && it.epoch == a.epoch && visible(it) && navigator.matches(request, it.document, it.epoch)
                    } == true
                    if (!valid()) return@withLock
                    val target = loadNavigationTarget(a.project, request)
                    currentCoroutineContext().ensureActive()
                    if (!valid()) return@withLock
                    controls.detach(); tones.detach(); review.detach(); reviewDetached = true
                    if (!views.detach()) report("The latest local view save was not confirmed. The project and original remain saved.")
                    viewOwner = null
                    try {
                        adoptDocument(a, target) // Already holding mutations.
                        attachment()?.let { bind(it); enter(it) }
                        activate(EditorWorkflowPanel.Documents, false)
                    } catch (failure: Throwable) {
                        // A failed switch still borrows the root's retained
                        // current document. Restore settings/metadata ownership;
                        // renderer/native failures stay with that root owner.
                        try { attachment()?.takeIf { it.project === a.project }?.let { bind(it); enter(it) } }
                        catch (cleanup: Throwable) { failure.addSuppressed(cleanup) }
                        throw failure
                    }
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { navigator.finish(request, "The document switch could not finish. The current project is retained; retry or reopen it.") }
            finally { navigator.finish(request) }
        }
        navigation = work
        work.invokeOnCompletion { if (navigation === work) navigation = null }
        work.start()
    }
    public fun cancelSwitch(binding: WorkflowBinding, epoch: Long): Unit {
        if (navigator.state.value.binding == binding && navigator.state.value.attachmentEpoch == epoch) {
            navigator.cancel(); navigation?.cancel()
        }
    }
    /** Signal cancellation BEFORE the parent waits for mutations. A navigation
     * producer may already own that gate while it awaits native retirement. */
    public fun beginDeparture(): Unit {
        departing=true;mutable.value=EditorWorkflowPanel.None;navigator.detach()
        navigation?.cancel();review.cancelOperation()
        controls.beginDeparture();tones.beginDeparture()
    }
    /** Cancel first, then join. Safe when the root already holds mutations: a
     * child still waiting for that gate is cancelled without reacquiring it. */
    public suspend fun detach(): Unit = withContext(NonCancellable) {
        beginDeparture()
        try {
            navigation?.cancelAndJoin(); navigation = null
            review.detach(); reviewDetached = true; controls.detach();tones.detach()
            if (!views.detach()) report("The latest local view save was not confirmed. Reload saved views to inspect it.")
            viewOwner = null; observed = null
        } finally { if(!closed)departing = false }
    }
    public suspend fun close(): Unit { closed=true;detach();controls.close();tones.close() }
}
