package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal data class AiAttachment(val project: WorkbenchProject, val document: DocumentSnapshot, val epoch: Long) {
    val binding: WorkflowBinding get() = document.render.revision.let {
        WorkflowBinding(it.projectId, document.documentId, it.hostSeq, it.stateHash)
    }
    fun sameProject(other: AiAttachment?): Boolean = other != null && project === other.project &&
        epoch == other.epoch && binding.projectId == other.binding.projectId && binding.documentId == other.binding.documentId
}
internal data class AiDraftOptions(val instruction: String, val estimate: AiTokenEstimate?, val feather: UInt = 8u,
    val allow16bitCopy: Boolean = false, val assumeUntaggedSrgb: Boolean = false)
internal enum class AiView { Before, After, Wipe, Blink, Split, Difference }
internal data class AiSavedResultLabel(val resultId: String, val label: String)
internal data class AiDisplayTile<T>(val region: AiRegion, val image: T)
internal data class AiDisplayFrame<T>(val candidateId: String, val binding: WorkflowBinding, val camera: Camera,
    val mode: AiCompareMode, val width: UInt, val height: UInt, val tiles: List<AiDisplayTile<T>>)
internal data class AiEditorState<T>(
    val open: Boolean = false, val context: AiContext? = null, val review: AiReview? = null,
    val candidate: AiCandidateInfo? = null, val receipt: AiResultReceipt? = null,
    val busy: AiStage = AiStage.Idle, val loadingPixels: Boolean = false, val stale: Boolean = false,
    val attempted: Boolean = false, val retrySave: Boolean = false, val message: String? = null,
    val view: AiView = AiView.After, val wipe: Float = .5f, val differenceGain: UByte = 1u,
    val blinkAfter: Boolean = true, val camera: Camera = Camera(Point(1.0, 1.0), 1.0, 0.0, 1.0, 1.0),
    val brush: Boolean = false, val radius: Double = 32.0, val subtract: Boolean = false,
    val clearNext: Boolean = true, val contact: List<Point> = emptyList(), val frame: AiDisplayFrame<T>? = null,
    val savedResults: List<AiSavedResultLabel> = emptyList(),
) {
    val idle: Boolean get() = busy == AiStage.Idle
}

/** The injectable boundary is the actual shared controller, not an alternative
 * image/proof implementation. It lets app ownership tests pause real handoffs. */
internal interface AiDriver {
    suspend fun prepare(options: AiPrepareOptions): AiReview
    suspend fun send(request: String, estimate: ULong, acknowledge: Boolean): AiCandidateInfo
    suspend fun recover(): AiCandidateInfo
    suspend fun pixels(mode: AiCompareMode, region: AiRegion): AiPixels
    suspend fun brush(value: AiAcceptanceBrush): AiCandidateInfo
    suspend fun save(value: AiSaveOptions): AiResultReceipt
    suspend fun close()
}
private class SharedAiDriver(project: WorkbenchProject, service: WorkbenchAiService, scope: CoroutineScope) : AiDriver {
    private val interaction = AiInteraction(project, service, scope)
    override suspend fun prepare(options: AiPrepareOptions) = interaction.prepare(options)
    override suspend fun send(request: String, estimate: ULong, acknowledge: Boolean) = interaction.send(request, estimate, acknowledge)
    override suspend fun recover() = interaction.recoverCandidate()
    override suspend fun pixels(mode: AiCompareMode, region: AiRegion) = interaction.pixels(mode, region)
    override suspend fun brush(value: AiAcceptanceBrush) = interaction.acceptBrush(value)
    override suspend fun save(value: AiSaveOptions) = interaction.save(value)
    override suspend fun close() = interaction.close()
}

/** UI-dispatcher confined. Exactly one explicit operation and one replaceable
 * tile reader exist. The reader is joined before an action reuses AiInteraction.
 * detach must settle before the editor closes its borrowed project. No service,
 * key, budget history or request is opened by constructing this controller. */
internal class AiEditorController<T>(
    private val scope: CoroutineScope,
    private val core: WorkbenchCore,
    private val service: AiServiceController,
    private val attachment: () -> AiAttachment?,
    private val mutationGate: Mutex,
    private val metadata: suspend (AiAttachment) -> WorkflowMetadata,
    private val refreshAfterMutation: suspend (AiAttachment) -> Unit,
    private val image: suspend (AiPixels) -> T,
    private val abandonImage: (T) -> Unit = {},
    private val results: (WorkbenchProject) -> WorkbenchAiResults = { it.aiResults() },
    private val driverFactory: (WorkbenchProject, WorkbenchAiService, CoroutineScope) -> AiDriver = ::SharedAiDriver,
    private val now: () -> Long = System::currentTimeMillis,
    private val canOperate: () -> Boolean = { true },
) {
    private val mutable = MutableStateFlow(AiEditorState<T>())
    val state: StateFlow<AiEditorState<T>> = mutable.asStateFlow()
    private var driver: AiDriver? = null
    private var owner: AiAttachment? = null
    private var action: Job? = null
    private val pixels = AiLatestTask(scope)
    private var ticket = 0L
    private var lifetime = 0L
    private var detaching = false
    private var closed = false
    private var retirements = 0
    private val retirement = Mutex()
    private val contact = AiContactCollector()
    private var saveOptions: Pair<String, AiSaveOptions>? = null

    private fun id(): String = core.newId(now().toULong())
    private fun same(value: AiAttachment): Boolean = value.sameProject(attachment()) && value.binding == attachment()?.binding
    private fun requireCurrent(): AiAttachment = attachment()?.takeIf { !closed && !detaching }
        ?: throw AiFailure(AiFailureKind.Closed)
    private fun message(value: String?) { mutable.value = mutable.value.copy(message = value) }
    private fun describe(error: Exception): String = when (error) {
        is AiFailure -> aiFailureText(error.kind)
        is WorkflowFailure -> when (error.kind) {
            WorkflowFailureKind.Stale -> aiFailureText(AiFailureKind.Stale)
            WorkflowFailureKind.Cancelled -> aiFailureText(AiFailureKind.Cancelled)
            else -> "The project could not complete this edit. Its last accepted revision is preserved."
        }
        is AiDisplayRefusal -> when (error.reason) {
            AiDisplayRefusal.Reason.ZoomIn -> "This view exceeds the 64 MiB tile limit. Zoom in to inspect original pixels."
            AiDisplayRefusal.Reason.ContactLimit -> "This contact is too long. Use several shorter acceptance strokes."
            AiDisplayRefusal.Reason.Stale -> "The comparison changed. Repeat the acceptance stroke."
            AiDisplayRefusal.Reason.Invalid -> "The requested full-resolution display is unavailable."
        }
        else -> "The AI operation is unavailable. Existing project and budget history are preserved."
    }

    fun open() {
        if (closed || detaching) return
        mutable.value = mutable.value.copy(open = true, savedResults = savedResults())
        reloadContext()
    }
    fun hide() {
        mutable.value = mutable.value.copy(open = false)
        cancelContact(); invalidatePixels()
    }
    fun needsDecision(): Boolean = (mutable.value.attempted || mutable.value.busy == AiStage.Sending) && mutable.value.receipt == null
    /** Explicit discard releases a retained paid/private result. Its durable
     * billing history stays intact. Merely hiding the panel never discards it. */
    fun discard() = launch(AiStage.Comparing) {
        withContext(NonCancellable) { driver?.close() }
        driver=null;owner=null;saveOptions=null
        mutable.value=AiEditorState(open=mutable.value.open,context=mutable.value.context,savedResults=savedResults(),
            message="Review discarded. Recorded or uncertain charges remain in private budget history.")
    }
    /** Lifecycle pause is explicit cancellation; it cannot promise a refund for
     * an already admitted provider request. Nothing is automatically resent. */
    fun background() { hide(); action?.cancel() }
    fun reloadContext() = launch(AiStage.Preparing) {
        val selected = requireCurrent()
        val value = results(selected.project).context(selected.binding)
        currentCoroutineContext().ensureActive()
        if (!same(selected) || value.binding != selected.binding) throw AiFailure(AiFailureKind.Stale)
        mutable.value = mutable.value.copy(context = value, stale = mutable.value.review?.binding?.let { it != value.binding } == true)
    }
    /** Call only after the editor admits its exact new render snapshot. */
    fun changed() {
        mutable.value = mutable.value.copy(savedResults = savedResults())
        val current = attachment()
        val review = mutable.value.review
        if (owner?.sameProject(current) == false || (review != null && review.binding != current?.binding)) {
            cancelContact(); invalidatePixels()
            mutable.value = mutable.value.copy(stale = true, context = null,
                message = "The project changed. Prepare a fresh review before Send or save. An existing paid attempt is not repeated.")
        } else if (mutable.value.context?.binding != current?.binding) {
            mutable.value = mutable.value.copy(context = null)
        }
    }

    private fun savedResults(): List<AiSavedResultLabel> = attachment()?.document?.let { doc ->
        doc.render.items.asSequence().filter { it.shape is Shape.Result }.take(65).map { item ->
            AiSavedResultLabel((item.shape as Shape.Result).resultId,
                doc.layers.firstOrNull { it.id == item.layerId }?.name ?: "Result")
        }.toList()
    }.orEmpty()

    /** This explicit status action uses the current visible CAS and does not
     * send to a provider. A failed/ambiguous call is refreshed, never retried
     * automatically with a new transaction. Undo can restore a hidden Result. */
    fun savedStatus(resultId: String, accepted: Boolean) = launch(AiStage.Saving) {
        val selected = requireCurrent()
        if (mutable.value.savedResults.none { it.resultId == resultId } && mutable.value.receipt?.resultId != resultId)
            throw AiFailure(AiFailureKind.Invalid)
        mutationGate.withLock {
            if (!same(selected)) throw AiFailure(AiFailureKind.Stale)
            try {
                results(selected.project).resultStatus(selected.binding, resultId, accepted, metadata(selected))
                message(if (accepted) "Result marked accepted." else "Result hidden. Undo can restore it.")
            } finally {
                withContext(NonCancellable) { if (selected.sameProject(attachment())) refreshAfterMutation(selected) }
            }
        }
    }

    fun prepare(draft: AiDraftOptions): Job? {
        if (needsDecision()) {
            message("Save the paid Result or explicitly discard its review before preparing another request.")
            return null
        }
        return launch(AiStage.Preparing) {
        if(!canOperate())throw AiFailure(AiFailureKind.Busy)
        if (draft.instruction.length > 16_384 || draft.instruction.encodeToByteArray().size > 16_384 || draft.feather > 64u)
            throw AiFailure(AiFailureKind.Invalid)
        val selected = requireCurrent()
        val configuration = service.state.value.configuration ?: throw AiFailure(AiFailureKind.Unsupported)
        val provider = service.requireService()
        val context = results(selected.project).context(selected.binding)
        if (!same(selected) || context.binding != selected.binding) throw AiFailure(AiFailureKind.Stale)
        // A paid/uncertain unsaved request was refused before this action. Old requests settle
        // before their native global request slot is acquired by a new one.
        withContext(NonCancellable) { driver?.close(); driver = null; owner = null }
        saveOptions = null
        mutable.value = mutable.value.copy(context = context, review = null, candidate = null, receipt = null,
            attempted = false, retrySave = false, stale = false, frame = null)
        val next = driverFactory(selected.project, provider, scope)
        var adopted = false
        try {
            val review = next.prepare(AiPrepareOptions(selected.binding, context.changeSelections, id(),
                draft.instruction, configuration.fingerprint, draft.estimate, draft.feather,
                draft.allow16bitCopy, draft.assumeUntaggedSrgb))
            currentCoroutineContext().ensureActive()
            if (!same(selected) || review.binding != selected.binding || review.sourceAssetId != context.sourceAssetId ||
                review.configurationFingerprint != configuration.fingerprint) throw AiFailure(AiFailureKind.Stale)
            driver = next; owner = selected; adopted = true
            mutable.value = mutable.value.copy(review = review, stale = false, clearNext = true)
        } finally { if (!adopted) withContext(NonCancellable) { next.close() } }
        }
    }

    /** Capture requestId and cost from the displayed review in the button's
     * callback. Never fetch a newer review after the user's Send activation. */
    fun send(requestId: String, displayedEstimate: ULong, acknowledge: Boolean) = launch(AiStage.Sending) {
        if(!canOperate())throw AiFailure(AiFailureKind.Busy)
        val selected = owner ?: throw AiFailure(AiFailureKind.Invalid)
        val review = mutable.value.review ?: throw AiFailure(AiFailureKind.Invalid)
        if (!same(selected) || mutable.value.stale || mutable.value.attempted ||
            review.requestId != requestId || review.estimatedMicrousd != displayedEstimate)
            throw AiFailure(AiFailureKind.Stale)
        mutable.value = mutable.value.copy(attempted = true)
        try {
            val candidate = checkNotNull(driver).send(requestId, displayedEstimate, acknowledge)
            if (candidate.requestId != review.requestId || candidate.binding != review.binding)
                throw AiFailure(AiFailureKind.Proof)
            mutable.value = mutable.value.copy(candidate = candidate, stale = !same(selected), receipt = null,
                clearNext = true, brush = false)
            fit()
        } catch (error: AiFailure) {
            if (error.kind in setOf(AiFailureKind.SoftBudget, AiFailureKind.Busy))
                mutable.value = mutable.value.copy(attempted = false)
            throw error
        }
        // Readiness is refreshed only by its explicit settings Refresh action;
        // a second owner operation here could hide the accepted candidate.
    }

    /** Reread the retained request only: no provider call and no new attempt. */
    fun recoverCandidate() = launch(AiStage.Comparing) {
        val selected=owner?:throw AiFailure(AiFailureKind.Invalid)
        val review=mutable.value.review?:throw AiFailure(AiFailureKind.Invalid)
        if(!selected.sameProject(attachment())||!mutable.value.attempted)throw AiFailure(AiFailureKind.Stale)
        val candidate=checkNotNull(driver).recover()
        if(candidate.requestId!=review.requestId||candidate.binding!=review.binding)throw AiFailure(AiFailureKind.Proof)
        mutable.value=mutable.value.copy(candidate=candidate,stale=!same(selected))
        fit()
    }

    fun view(value: AiView) {
        cancelContact()
        val oldWidth = canvasWidth()
        mutable.value = mutable.value.copy(view = value, blinkAfter = true, frame = null)
        if (oldWidth != canvasWidth()) fit() else requestPixels()
    }
    fun wipe(value: Float) { if (value.isFinite()) { cancelContact(); mutable.value = mutable.value.copy(wipe = value.coerceIn(0f, 1f)); requestPixels() } }
    fun difference(value: Int) { cancelContact(); mutable.value = mutable.value.copy(differenceGain = value.coerceIn(1, 16).toUByte()); requestPixels() }
    fun blink() { if (mutable.value.view == AiView.Blink && mutable.value.idle && !contact.active) { mutable.value = mutable.value.copy(blinkAfter = !mutable.value.blinkAfter); requestPixels() } }
    private fun mode(): AiCompareMode = when (mutable.value.view) {
        AiView.Before -> AiCompareMode.Before; AiView.After -> AiCompareMode.After
        AiView.Blink -> if (mutable.value.blinkAfter) AiCompareMode.After else AiCompareMode.Before
        AiView.Split -> AiCompareMode.Split
        AiView.Difference -> AiCompareMode.Difference(mutable.value.differenceGain)
        AiView.Wipe -> AiCompareMode.Wipe(true, ((mutable.value.candidate?.width ?: 0u).toDouble() * mutable.value.wipe).toLong().coerceAtLeast(0).toUInt())
    }
    private fun canvasWidth(): UInt = mutable.value.candidate?.width?.let { aiCanvasWidth(it, mode()) } ?: 1u
    fun viewport(width: Double, height: Double) {
        if (!width.isFinite() || !height.isFinite() || width <= 0 || height <= 0) return
        val before = mutable.value.camera
        if (before.viewportWidth == width && before.viewportHeight == height) return
        camera(before.copy(viewportWidth = width, viewportHeight = height))
        if (before.viewportWidth <= 1 || before.viewportHeight <= 1) fit()
    }
    fun fit() {
        val value = mutable.value.candidate ?: return
        val current = mutable.value.camera
        val width = canvasWidth().toDouble(); val height = value.height.toDouble()
        camera(current.copy(center = Point(width / 2, height / 2), rotation = 0.0,
            scale = minOf(current.viewportWidth / width, current.viewportHeight / height).coerceIn(.001, 256.0)))
    }
    fun zoom(factor: Double, at: Point? = null) {
        if (!factor.isFinite() || factor <= 0) return
        val old = mutable.value.camera
        val point = at ?: Point(old.viewportWidth / 2, old.viewportHeight / 2)
        val before = core.mapPoints(old, true, listOf(point)).single()
        val next = old.copy(scale = (old.scale * factor).coerceIn(.001, 256.0))
        val after = core.mapPoints(next, true, listOf(point)).single()
        camera(next.copy(center = Point(next.center.x + before.x - after.x, next.center.y + before.y - after.y)))
    }
    fun pan(dx: Double, dy: Double) {
        if (!dx.isFinite() || !dy.isFinite()) return
        val old = mutable.value.camera
        val points = core.mapPoints(old, true, listOf(Point(0.0, 0.0), Point(dx, dy)))
        camera(old.copy(center = Point(old.center.x - points[1].x + points[0].x, old.center.y - points[1].y + points[0].y)))
    }
    private fun camera(value: Camera) { cancelContact(); mutable.value = mutable.value.copy(camera = value); requestPixels() }

    fun brush(enabled: Boolean) {
        cancelContact()
        mutable.value = mutable.value.copy(brush = enabled, view = if (enabled && mutable.value.view == AiView.Blink) AiView.After else mutable.value.view)
        requestPixels()
    }
    fun radius(value: Double) { if (value.isFinite()) { cancelContact(); mutable.value = mutable.value.copy(radius = value.coerceIn(.5, 256.0)) } }
    fun subtract(value: Boolean) { cancelContact(); mutable.value = mutable.value.copy(subtract = value) }
    fun clearNext(value: Boolean) { cancelContact(); mutable.value = mutable.value.copy(clearNext = value) }
    fun contactDown(point: Point): Boolean {
        val s = mutable.value; val candidate = s.candidate ?: return false
        val frame = s.frame ?: return false
        if (!s.brush || !s.idle || s.stale || s.retrySave || frame.candidateId != candidate.candidateId ||
            frame.camera != s.camera || frame.mode != mode()) return false
        return try {
            contact.begin(candidate.candidateId, s.camera, candidate.width, candidate.height, s.view == AiView.Split,
                core.mapPoints(s.camera, true, listOf(point)).single(), s.radius, s.subtract, s.clearNext)
            mutable.value = mutable.value.copy(contact = contact.points); true
        } catch (error: Exception) { message(describe(error)); false }
    }
    fun contactMove(points: List<Point>) {
        if (!contact.active) return
        try {
            if (points.size > AI_CONTACT_POINTS) throw AiDisplayRefusal(AiDisplayRefusal.Reason.ContactLimit)
            contact.append(mutable.value.candidate?.candidateId.orEmpty(), mutable.value.camera,
                core.mapPoints(mutable.value.camera, true, points))
            mutable.value = mutable.value.copy(contact = contact.points)
        } catch (error: Exception) { cancelContact(); message(describe(error)) }
    }
    fun contactUp() {
        val value = try { contact.finish(mutable.value.candidate?.candidateId.orEmpty(), mutable.value.camera) }
        catch (error: Exception) { message(describe(error)); null }
        mutable.value = mutable.value.copy(contact = emptyList())
        if (value != null) launch(AiStage.Accepting) {
            val selected = owner ?: throw AiFailure(AiFailureKind.Invalid)
            if (!same(selected) || mutable.value.stale || value.candidateId != mutable.value.candidate?.candidateId)
                throw AiFailure(AiFailureKind.Stale)
            val candidate = checkNotNull(driver).brush(AiAcceptanceBrush(value.candidateId, id(), value.points,
                value.radius, 255u, value.subtract, value.clearFirst))
            if (candidate.binding != selected.binding) throw AiFailure(AiFailureKind.Proof)
            mutable.value = mutable.value.copy(candidate = candidate, receipt = null, clearNext = false, stale = !same(selected))
        }
    }
    fun cancelContact() { contact.cancel(); mutable.value = mutable.value.copy(contact = emptyList()) }

    fun save() = launch(AiStage.Saving) {
        val selected = owner ?: throw AiFailure(AiFailureKind.Invalid)
        val candidate = mutable.value.candidate ?: throw AiFailure(AiFailureKind.Invalid)
        mutationGate.withLock {
            if (!selected.sameProject(attachment())) throw AiFailure(AiFailureKind.Stale)
            val prior = saveOptions
            val options = if (prior != null) {
                if (prior.first != candidate.candidateId) throw AiFailure(AiFailureKind.Stale)
                prior.second
            } else {
                if (!same(selected) || mutable.value.stale) throw AiFailure(AiFailureKind.Stale)
                AiSaveOptions(selected.binding, metadata(selected), id(), id(), id()).also {
                    saveOptions = candidate.candidateId to it
                    mutable.value = mutable.value.copy(retrySave = true)
                }
            }
            try {
                val receipt = checkNotNull(driver).save(options)
                // Accepted receipt bookkeeping precedes any refresh that might
                // fail or observe a newer remote revision.
                mutable.value = mutable.value.copy(receipt = receipt, retrySave = false)
            } finally {
                withContext(NonCancellable) {
                    if (selected.sameProject(attachment())) refreshAfterMutation(selected)
                }
            }
        }
    }

    fun cancel() { cancelContact(); invalidatePixels(); action?.cancel() }
    private fun invalidatePixels() { ticket++; pixels.cancelPending(); mutable.value = mutable.value.copy(frame = null, loadingPixels = false) }
    private fun launch(stage: AiStage, body: suspend () -> Unit): Job? {
        if (closed || detaching || !scope.isActive) return null
        if (action?.isCompleted == false) { message(aiFailureText(AiFailureKind.Busy)); return null }
        cancelContact(); invalidatePixels()
        val entered = lifetime
        mutable.value = mutable.value.copy(busy = stage, message = null)
        val job = scope.launch(start = CoroutineStart.LAZY) {
            try { pixels.pause(); body() }
            catch (cancel: CancellationException) { if (entered == lifetime && !detaching) message(aiFailureText(AiFailureKind.Cancelled)); throw cancel }
            catch (error: Exception) { if (entered == lifetime && !detaching) message(describe(error)) }
        }
        action = job
        // Cancellation makes isActive false before native producer cleanup has
        // settled. Retain ownership until completion, including child cleanup;
        // only this exact job may clear its slot or restart pixel work.
        job.invokeOnCompletion {
            if (entered == lifetime && action === job && !detaching) {
                action = null; mutable.value = mutable.value.copy(busy = AiStage.Idle)
                requestPixels()
            }
        }
        job.start(); return job
    }
    private fun requestPixels() {
        val s = mutable.value
        if (!s.open || !s.idle || s.stale || closed || detaching || action?.isCompleted == false) return
        val candidate = s.candidate ?: return
        val owner = owner ?: return
        val driver = driver ?: return
        val chosen = mode(); val camera = s.camera; val entered = ++ticket; val epoch = lifetime
        mutable.value = mutable.value.copy(frame = null, loadingPixels = true)
        pixels.offer {
            val acquired = ArrayList<AiDisplayTile<T>>()
            var published = false
            try {
                val width = aiCanvasWidth(candidate.width, chosen)
                val corners = core.mapPoints(camera, true, listOf(Point(0.0, 0.0), Point(camera.viewportWidth, 0.0),
                    Point(camera.viewportWidth, camera.viewportHeight), Point(0.0, camera.viewportHeight)))
                val plan = aiTilePlan(width, candidate.height, corners)
                for (region in plan.regions) {
                    currentCoroutineContext().ensureActive()
                    val value = driver.pixels(chosen, region)
                    if (value.candidateId != candidate.candidateId || value.region != region ||
                        value.canvasWidth != width || value.canvasHeight != candidate.height) throw AiFailure(AiFailureKind.Proof)
                    aiCheckPixels(value.region, value.canvasWidth, value.canvasHeight, value.rgbaSrgb)
                    val bitmap = image(value)
                    // No cancellable dispatcher handoff after ownership return.
                    acquired += AiDisplayTile(region, bitmap)
                }
                currentCoroutineContext().ensureActive()
                if (epoch != lifetime || entered != ticket || !same(owner) || !mutable.value.open || mutable.value.stale ||
                    mutable.value.candidate?.candidateId != candidate.candidateId || mutable.value.camera != camera || mode() != chosen) return@offer
                mutable.value = mutable.value.copy(frame = AiDisplayFrame(candidate.candidateId, candidate.binding,
                    camera, chosen, width, candidate.height, acquired.toList()), loadingPixels = false)
                published = true
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { if (epoch == lifetime && entered == ticket) message(describe(error)) }
            finally {
                if (!published) acquired.forEach { abandonImage(it.image) }
                if (epoch == lifetime && entered == ticket) mutable.value = mutable.value.copy(loadingPixels = false)
            }
        }
    }

    suspend fun detach() {
        retirements++; detaching = true; lifetime++; invalidatePixels(); cancelContact()
        withContext(NonCancellable) { retirement.withLock {
            action?.cancelAndJoin(); action = null
            pixels.pause()
            try { driver?.close() }
            finally { driver = null; owner = null; saveOptions = null; mutable.value = AiEditorState(); retirements--; detaching = closed || retirements != 0 }
        } }
    }
    suspend fun close() { closed = true; try { detach() } finally { pixels.close() } }
}
