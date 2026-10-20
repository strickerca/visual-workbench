package com.visualworkbench.desktop

import com.visualworkbench.desktop.stream.DesktopStreamContact
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test

/** Real common annotation state machine, with narrow in-memory platform ports.
 * No renderer, GUI, native library, provider or device operation executes. */
class DesktopStreamSessionTest {
    @Test fun synchronousOwnerDoesNotAcquireAViewUntilStart() = desktopStreamSessionFixture { f ->
        assertSame(f.annotation, f.session.annotation)
        assertTrue(f.views.isEmpty())
        assertNull(f.session.state.value.view)
        f.session.start()
        assertSame(f.views.single(), f.session.state.value.view)
        assertFalse(f.session.state.value.busy)
        assertEquals(0, f.receiver.contacts)
    }

    @Test fun cancelledStartJoinsAndRetainsALateFactoryBeforeClosingNativeParents() = desktopStreamSessionFixture { f ->
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.onView = { view, retain ->
            entered.complete(Unit)
            desktopStreamSessionLate(release)
            retain(view)
            view
        }
        val starting = launch { f.session.start() }
        entered.await()
        starting.cancel()
        val closing = async(start = CoroutineStart.UNDISPATCHED) { f.session.close() }
        assertFalse(closing.isCompleted)
        assertEquals(0, f.receiver.closes)
        release.complete(Unit)
        starting.join(); closing.await()
        assertEquals(1, f.views.single().closes)
        assertEquals(1, f.receiver.closes)
        assertTrue(f.events.indexOf("view1:close") < f.events.indexOf("stream:close"))
        assertTrue(f.session.state.value.closed)
        assertTrue(f.parentJob.children.none())
    }

    @Test fun freezeJoinsActualStopAndChecksTheFullDisplayedFrameBeforeBorrowing() = desktopStreamSessionFixture { f ->
        f.session.start()
        val view = f.views.single()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        view.stopAction = { entered.complete(Unit); release.await() }
        val freezing = async { f.session.freeze(Point(30.0, 40.0), f.camera) }
        entered.await()
        assertEquals(0, f.receiver.contacts)
        assertTrue(f.editors.isEmpty())
        release.complete(Unit)
        val child = freezing.await()
        assertSame(f.editors.single(), child)
        assertSame(f.drafts.single().project, f.editors.single().project)
        assertEquals(streamContactCamera(f.receiver.frame, f.camera.viewportWidth, f.camera.viewportHeight), f.editors.single().camera)
        assertEquals(Point(30.0, 40.0), view.pointer)
        assertEquals(Point(3.0, 4.0), f.session.state.value.contact!!.point)
        assertEquals(f.annotation.state.value.draft!!.frame, f.session.state.value.contact!!.frame)
        assertTrue(f.events.indexOf("view1:stop") < f.events.indexOf("stream:contact"))
        assertTrue(f.events.indexOf("stream:contact") < f.events.indexOf("editor1:open"))
        assertSame(child, f.session.freeze(Point(99.0, 99.0), f.camera.copy(scale = 3.0)))
        assertEquals(1, view.stops)
        assertEquals(1, f.editors.size)
    }

    @Test fun lastSettledFrameDeterminesCameraInsteadOfTheFrameSampledBeforeStop() = desktopStreamSessionFixture { f ->
        f.session.start()
        val old = f.receiver.frame
        val template = streamContactCamera(old, 600.0, 400.0)
        f.views.single().stopAction = {
            f.receiver.frame = old.copy(frameId = 2uL, width = 16u)
            f.views.single().contactFrame = f.receiver.frame
        }
        f.session.freeze(Point(10.0, 12.0), template)
        val actual = streamContactCamera(f.receiver.frame, 600.0, 400.0)
        assertEquals(actual, f.editors.single().camera)
        assertNotEquals(template, actual)
        assertEquals(f.receiver.frame, f.session.state.value.contact!!.frame)
    }

    @Test fun computerRegionRequestIsOwnedAndJoinedBeforeReceiverClose() = desktopStreamSessionFixture { f ->
        f.session.start()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.onRegion = { entered.complete(Unit); desktopStreamSessionLate(release) }
        val requesting = launch { f.session.requestComputerRegion() }
        entered.await()
        val closing = async(start = CoroutineStart.UNDISPATCHED) { f.session.close() }
        assertFalse(closing.isCompleted)
        assertEquals(0, f.views.single().closes)
        assertEquals(0, f.receiver.closes)
        release.complete(Unit)
        requesting.join(); closing.await()
        assertEquals(1, f.regionRequests)
        assertEquals(0, f.receiver.contacts)
        assertEquals(1, f.receiver.closes)
    }

    @Test fun matchingFrameIdCannotHideDifferentDisplayedGeometry() = desktopStreamSessionFixture { f ->
        f.session.start()
        f.views.single().contactFrame = f.receiver.frame.copy(width = 9u)
        assertTrue(runCatching { f.session.freeze(Point(1.0, 2.0), f.camera) }.isFailure)
        assertTrue(f.annotation.state.value.editorFailed)
        assertNotNull(f.annotation.state.value.draft)
        assertTrue(f.editors.isEmpty())
        assertEquals(0, f.drafts.single().closes)
        f.session.resume()
        assertEquals(1, f.drafts.single().closes)
        assertEquals(1, f.receiver.resumes)
        assertEquals(2, f.views.size)
    }

    @Test fun rejectedContactRenewsOnlyTheStoppedRenderer() = desktopStreamSessionFixture { f ->
        f.session.start()
        f.views.single().contactAccepted = false
        assertNull(f.session.freeze(Point(-1.0, -1.0), f.camera))
        assertEquals(1, f.views.first().closes)
        assertEquals(2, f.views.size)
        assertEquals(0, f.receiver.contacts)
        assertEquals(0, f.receiver.resumes)
        assertNull(f.annotation.state.value.draft)
        assertSame(f.views.last(), f.session.state.value.view)
    }

    @Test fun cancelledEditorFactoryKeepsItsLateBorrowUntilChildClosePrecedesDraftClose() = desktopStreamSessionFixture { f ->
        f.session.start()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.onEditor = { editor, retain -> retain(editor); entered.complete(Unit); desktopStreamSessionLate(release); editor }
        val freezing = launch { f.session.freeze(Point(1.0, 2.0), f.camera) }
        entered.await()
        val held = f.editors.single()
        assertSame(held, f.session.state.value.child)
        val closing = async(start = CoroutineStart.UNDISPATCHED) { f.session.close() }
        assertFalse(closing.isCompleted)
        assertEquals(0, held.closes)
        assertEquals(0, f.drafts.single().closes)
        release.complete(Unit)
        freezing.join(); closing.await()
        assertEquals(1, held.closes)
        assertTrue(f.events.indexOf("editor1:close") < f.events.indexOf("draft1:close"))
        assertEquals(1, f.drafts.single().project.closes)
        assertEquals(0, f.target.closes)
        assertEquals(0, f.link.closes)
    }

    @Test fun failedEditorReadinessRetainsTheChildUntilExplicitResume() = desktopStreamSessionFixture { f ->
        f.onEditor = { editor, retain -> retain(editor); editor.readinessFailure = IllegalStateException("failed initialization"); editor }
        f.session.start()
        assertTrue(runCatching { f.session.freeze(Point(1.0, 2.0), f.camera) }.isFailure)
        assertSame(f.editors.single(), f.session.state.value.child)
        assertTrue(f.annotation.state.value.editorFailed)
        assertTrue(f.session.state.value.cleanupPending)
        assertTrue(runCatching { f.session.save() }.isFailure)
        assertEquals(0, f.drafts.single().prepares)
        f.session.resume()
        assertEquals(1, f.editors.single().closes)
        assertEquals(1, f.drafts.single().closes)
        assertNull(f.session.state.value.child)
        assertFalse(f.annotation.state.value.editorFailed)
        assertFalse(f.session.state.value.cleanupPending)
    }

    @Test fun refusedEditSealNeverPreparesSaveAndLaterSaveUsesTheSameDraft() = desktopStreamSessionFixture { f ->
        f.openDraft()
        val editor = f.editors.single()
        editor.sealFailure = true
        assertTrue(runCatching { f.session.save() }.isFailure)
        assertEquals(0, f.drafts.single().prepares)
        assertFalse(f.annotation.state.value.saveRetry)
        editor.sealFailure = false
        f.lossless()
        val receipt = f.session.save()
        assertEquals(receipt, f.annotation.state.value.saved)
        assertEquals(2, editor.seals)
        assertEquals(1, f.drafts.single().prepares)
        assertEquals(1, f.drafts.single().plan.commits)
        assertEquals(0, f.target.closes)
    }

    @Test fun originalObserverAdoptsOnlyTheExactDraftWithoutReplacingTheChild() = desktopStreamSessionFixture { f ->
        f.openDraft()
        val draft = f.annotation.state.value.draft
        val child = f.session.state.value.child
        f.lossless()
        assertTrue(f.annotation.state.value.lossless)
        assertEquals(draft, f.annotation.state.value.draft)
        assertSame(child, f.session.state.value.child)
        assertEquals(1, f.editors.size)
        assertEquals(1, f.views.single().adoptions)
        assertEquals(streamContactCamera(f.receiver.frame, f.camera.viewportWidth, f.camera.viewportHeight), f.editors.single().camera)
        assertEquals(0, f.pixels.single().closes)
    }

    @Test fun wrongOriginalCannotUpgradeOrAdoptAndItsLeaseIsClosed() = desktopStreamSessionFixture { f ->
        f.openDraft()
        f.receiver.pixelFrame = f.receiver.frame.copy(sourceGeneration = 99uL)
        f.receiver.losslessReady = true
        f.receiver.emit(CaptureStreamStage.FrozenLossless)
        f.session.state.first { it.cleanupPending }
        assertFalse(f.annotation.state.value.lossless)
        assertEquals(0, f.views.single().adoptions)
        assertEquals(1, f.pixels.single().closes)
        assertEquals(0, f.editors.single().closes)
        assertNotNull(f.annotation.state.value.draft)
    }

    @Test fun unknownSaveRetriesTheExactPlanAfterAttachmentLossAndForbidsResume() = desktopStreamSessionFixture { f ->
        f.openDraft(); f.lossless()
        val draft = f.drafts.single()
        draft.plan.failCommits = 1
        assertTrue(runCatching { f.session.save() }.isFailure)
        assertTrue(f.annotation.state.value.saveRetry)
        assertEquals(1, draft.prepares)
        assertEquals(1, f.editors.single().seals)
        f.attached = false
        f.receiver.epoch++
        f.receiver.emit(CaptureStreamStage.Closed, available = false)
        assertTrue(runCatching { f.session.resume() }.isFailure)
        assertEquals(0, f.views.single().closes)
        assertEquals(0, f.editors.single().closes)
        assertEquals(0, f.receiver.resumes)
        val receipt = f.session.save()
        assertEquals(2, draft.plan.commits)
        assertEquals(1, draft.prepares)
        assertEquals(1, f.editors.single().seals)
        assertEquals(receipt, f.session.save())
        assertEquals(2, draft.plan.commits)
        assertFalse(f.annotation.state.value.saveRetry)
    }

    @Test fun closedSourceRetainsPixelsButCannotPrepareAFreshSave() = desktopStreamSessionFixture { f ->
        f.openDraft(); f.lossless()
        val view = f.views.single()
        val child = f.editors.single()
        f.receiver.emit(CaptureStreamStage.SourceClosed, available = false)
        f.session.state.first { it.signal.stage == CaptureStreamStage.SourceClosed }
        assertSame(view, f.session.state.value.view)
        assertSame(child, f.session.state.value.child)
        assertTrue(f.annotation.state.value.lossless)
        assertEquals(0, view.closes)
        assertEquals(0, child.closes)
        assertTrue(runCatching { f.session.save() }.isFailure)
        assertEquals(0, child.seals)
        assertEquals(0, f.drafts.single().prepares)
        assertEquals(0, f.pixels.single().closes)
    }

    @Test fun explicitDegradedSaveUsesOnlyTheTimedOutPreview() = desktopStreamSessionFixture { f ->
        f.openDraft()
        f.receiver.previewReady = true
        f.receiver.emit(CaptureStreamStage.FrozenDegraded)
        assertTrue(runCatching { f.session.save() }.isFailure)
        assertEquals(0, f.drafts.single().prepares)
        val receipt = f.session.save(allowDegraded = true)
        assertEquals(receipt, f.annotation.state.value.saved)
        assertTrue(f.annotation.state.value.degradedSaved)
        assertEquals(1, f.drafts.single().degradedPrepares)
        assertNull(f.drafts.single().plan.info.originalAssetId)
        assertEquals(1, f.pixels.single().closes)
    }

    @Test fun resumeRetiresAllOldOwnersBeforeCreatingAFreshView() = desktopStreamSessionFixture { f ->
        f.openDraft()
        val old = f.views.single()
        f.session.resume()
        assertEquals(1, old.closes)
        assertEquals(1, f.editors.single().closes)
        assertEquals(1, f.drafts.single().closes)
        assertEquals(1, f.receiver.resumes)
        assertEquals(2, f.views.size)
        assertNotSame(old, f.session.state.value.view)
        assertNull(f.session.state.value.child)
        assertNull(f.session.state.value.contact)
        assertNull(f.annotation.state.value.draft)
        assertTrue(f.events.indexOf("view1:close") < f.events.indexOf("editor1:close"))
        assertTrue(f.events.indexOf("editor1:close") < f.events.indexOf("draft1:close"))
        assertTrue(f.events.indexOf("draft1:close") < f.events.indexOf("stream:resume"))
        assertTrue(f.events.indexOf("stream:resume") < f.events.indexOf("view2:open"))
        assertEquals(0, f.target.closes)
    }

    @Test fun failedRendererCloseRetainsTheExactOwnerAndBlocksLaterRetirementStages() = desktopStreamSessionFixture { f ->
        f.openDraft()
        val held = f.views.single()
        held.closeFailure = true
        assertTrue(runCatching { f.session.resume() }.isFailure)
        assertSame(held, f.session.state.value.view)
        assertSame(f.editors.single(), f.session.state.value.child)
        assertTrue(f.session.state.value.cleanupPending)
        assertEquals(0, f.editors.single().closes)
        assertEquals(0, f.drafts.single().closes)
        assertEquals(0, f.receiver.resumes)
        assertEquals(1, f.views.size)
        assertTrue(runCatching { f.session.close() }.isFailure)
        assertSame(held, f.session.state.value.view)
        assertEquals(0, f.receiver.closes)
    }

    @Test fun retryAfterChildCloseFailureNeverReclosesTheRetiredView() = desktopStreamSessionFixture { f ->
        f.openDraft()
        val child = f.editors.single()
        child.closeFailure = true
        assertTrue(runCatching { f.session.resume() }.isFailure)
        assertNull(f.session.state.value.view)
        assertSame(child, f.session.state.value.child)
        assertEquals(1, f.views.single().closes)
        assertEquals(0, f.drafts.single().closes)
        child.closeFailure = false
        f.session.resume()
        assertEquals(1, f.views.first().closes)
        assertEquals(2, child.closes)
        assertEquals(1, f.drafts.single().closes)
        assertEquals(1, f.receiver.resumes)
        assertEquals(2, f.views.size)
    }

    @Test fun retryAfterNativeDraftCloseFailureKeepsCompletedPlatformStagesRetired() = desktopStreamSessionFixture { f ->
        f.openDraft()
        val draft = f.drafts.single()
        draft.closeFailure = true
        assertTrue(runCatching { f.session.resume() }.isFailure)
        assertNull(f.session.state.value.view)
        assertNull(f.session.state.value.child)
        assertEquals(0, f.receiver.resumes)
        draft.closeFailure = false
        f.session.resume()
        assertEquals(1, f.views.first().closes)
        assertEquals(1, f.editors.single().closes)
        assertEquals(2, draft.closes)
        assertEquals(1, draft.project.closes)
        assertEquals(1, f.receiver.resumes)
    }

    @Test fun failedFreshViewAfterResumeDoesNotRepeatTheNativeResume() = desktopStreamSessionFixture { f ->
        f.openDraft()
        f.onView = { view, retain -> retain(view); error("retained renderer factory") }
        assertTrue(runCatching { f.session.resume() }.isFailure)
        assertEquals(1, f.receiver.resumes)
        assertSame(f.views.last(), f.session.state.value.view)
        f.onView = { view, retain -> retain(view); view }
        f.session.resume()
        assertEquals(1, f.receiver.resumes)
        assertEquals(3, f.views.size)
        assertEquals(1, f.views[1].closes)
        assertSame(f.views.last(), f.session.state.value.view)
    }

    @Test fun closeJoinsOriginalAdoptionBeforeReleasingItsRendererOrNativeLease() = desktopStreamSessionFixture { f ->
        f.openDraft()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.views.single().adoptAction = { entered.complete(Unit); desktopStreamSessionLate(release) }
        f.receiver.losslessReady = true
        f.receiver.emit(CaptureStreamStage.FrozenLossless)
        entered.await()
        val closing = async(start = CoroutineStart.UNDISPATCHED) { f.session.close() }
        assertFalse(closing.isCompleted)
        assertEquals(0, f.views.single().closes)
        assertEquals(0, f.pixels.single().closes)
        release.complete(Unit)
        closing.await()
        assertEquals(1, f.views.single().closes)
        assertEquals(1, f.pixels.single().closes)
        assertEquals(1, f.drafts.single().closes)
        assertTrue(f.parentJob.children.none())
    }

    @Test fun cancelledConcurrentCloseWaitsForOneActualRetirementAndThenIsIdempotent() = desktopStreamSessionFixture { f ->
        f.openDraft()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.views.single().closeAction = { entered.complete(Unit); release.await() }
        val first = launch(start = CoroutineStart.UNDISPATCHED) { f.session.close() }
        entered.await()
        val second = launch(start = CoroutineStart.UNDISPATCHED) { f.session.close() }
        first.cancel(); second.cancel()
        assertFalse(first.isCompleted)
        assertFalse(second.isCompleted)
        assertEquals(0, f.receiver.closes)
        release.complete(Unit)
        joinAll(first, second)
        f.session.close()
        assertEquals(1, f.views.single().closes)
        assertEquals(1, f.editors.single().closes)
        assertEquals(1, f.receiver.closes)
        assertTrue(f.parentJob.children.none())
        assertTrue(f.session.state.value.closed)
        assertFalse(f.session.state.value.busy)
        assertTrue(runCatching { f.session.start() }.isFailure)
        assertEquals(1, f.views.size)
    }
}

private fun desktopStreamSessionFixture(body: suspend CoroutineScope.(DesktopStreamSessionTestFixture) -> Unit) = runBlocking {
    withTimeout(7000) {
        val fixture = DesktopStreamSessionTestFixture(this)
        try { body(fixture) } finally {
            withContext(NonCancellable) {
                fixture.gates.forEach { it.complete(Unit) }
                fixture.views.forEach { it.closeFailure = false }
                fixture.editors.forEach { it.closeFailure = false }
                fixture.drafts.forEach { it.closeFailure = false }
                fixture.receiver.closeFailure = false
                fixture.session.close()
                fixture.parentJob.cancelAndJoin()
            }
        }
    }
}

private class DesktopStreamSessionTestFixture(parent: CoroutineScope) {
    val parentJob = SupervisorJob(parent.coroutineContext[Job])
    val parentScope = CoroutineScope(parent.coroutineContext + parentJob)
    val events = mutableListOf<String>()
    val gates = mutableListOf<CompletableDeferred<Unit>>()
    val target = DesktopSessionTestProject("paired", "target-document", events)
    val link = DesktopSessionTestLink()
    val pixels = mutableListOf<DesktopSessionTestPixels>()
    val receiver = DesktopSessionTestStream(events, pixels)
    val drafts = mutableListOf<DesktopSessionTestDraft>()
    val views = mutableListOf<DesktopSessionTestRender>()
    val editors = mutableListOf<DesktopSessionTestEditor>()
    val camera = Camera(Point(7.0, 8.0), 1.5, 0.23, 600.0, 400.0)
    var attached = true
    var regionRequests = 0
    var onRegion: suspend () -> Unit = {}
    private var next = 0
    val annotation = CaptureAnnotationSession(target, "target-document", link, receiver,
        { "id-${next++}" }, { WorkflowMetadata("transaction", "device", it.nextLamport, 2000L) }, { attached },
        { _, _, document, _ ->
            DesktopSessionTestDraft(drafts.size + 1, CaptureDraftInfo(checkNotNull(receiver.receipt).draftId,
                document, receiver.frame, "a".repeat(64), receiver.epoch), events).also { drafts += it }
        }, requestRegion = { regionRequests++; onRegion() })
    var onView: suspend (DesktopSessionTestRender, (DesktopStreamRenderPort) -> Unit) -> DesktopStreamRenderPort = { view, retain -> retain(view); view }
    var onEditor: suspend (DesktopSessionTestEditor, (DesktopStreamEditorPort) -> Unit) -> DesktopStreamEditorPort = { editor, retain -> retain(editor); editor }
    val session = DesktopStreamSession(parentScope, annotation, { attached },
        openView = { _, retain ->
            val view = DesktopSessionTestRender(views.size + 1, receiver.frame, events)
            views += view; events += "view${views.size}:open"
            onView(view, retain)
        }, openEditor = { _, project, _, initialCamera, retain ->
            val editor = DesktopSessionTestEditor(editors.size + 1, project, initialCamera, events)
            editors += editor; events += "editor${editors.size}:open"
            onEditor(editor, retain)
        })
    fun gate() = CompletableDeferred<Unit>().also { gates += it }
    suspend fun openDraft() { session.start(); checkNotNull(session.freeze(Point(10.0, 12.0), camera)) }
    suspend fun lossless() {
        val view = views.last()
        receiver.losslessReady = true
        receiver.emit(CaptureStreamStage.FrozenLossless)
        view.adopted.await()
    }
}

private suspend fun desktopStreamSessionLate(release: CompletableDeferred<Unit>) {
    try { release.await() }
    catch (_: CancellationException) { withContext(NonCancellable) { release.await() } }
}

private class DesktopSessionTestRender(private val number: Int, var contactFrame: CaptureStreamFrame,
    private val events: MutableList<String>) : DesktopStreamRenderPort {
    var contactAccepted = true
    var pointer: Point? = null
    var stops = 0
    var adoptions = 0
    var closes = 0
    var closeFailure = false
    val adopted = CompletableDeferred<Unit>()
    var stopAction: suspend () -> Unit = {}
    var adoptAction: suspend () -> Unit = {}
    var closeAction: suspend () -> Unit = {}
    override suspend fun stopForContact(viewPointLogical: Point): DesktopStreamContact? {
        stops++; pointer = viewPointLogical; events += "view$number:stop"; stopAction()
        return if (contactAccepted) DesktopStreamContact(contactFrame, Point(3.0, 4.0)) else null
    }
    override suspend fun adoptLossless(annotation: CaptureAnnotationSession): Boolean {
        adoptions++; events += "view$number:adopt"; adoptAction()
        require(annotation.state.value.lossless && annotation.state.value.draft!!.frame == contactFrame)
        adopted.complete(Unit); return true
    }
    override suspend fun close() {
        closes++; events += "view$number:close"; closeAction()
        if (closeFailure) error("renderer retirement remains failed")
    }
}

private class DesktopSessionTestEditor(private val number: Int, val project: WorkbenchProject, val camera: Camera,
    private val events: MutableList<String>) : DesktopStreamEditorPort {
    var readinessFailure: Throwable? = null
    override val ready: Boolean get() { readinessFailure?.let { throw it }; return true }
    var seals = 0
    var closes = 0
    var sealFailure = false
    var closeFailure = false
    override suspend fun sealEdits() { seals++; events += "editor$number:seal"; if (sealFailure) error("active contact") }
    override suspend fun close() { closes++; events += "editor$number:close"; if (closeFailure) error("borrowed editor retained") }
}

private class DesktopSessionTestStream(private val events: MutableList<String>,
    private val pixels: MutableList<DesktopSessionTestPixels>) : WorkbenchCaptureStream {
    var frame = CaptureStreamFrame("session", 1uL, 1uL, 8u, 8u, 10, 20, 1.25, 3u, 10L, 1000L, "windows")
    var epoch = 7uL
    var contacts = 0
    var resumes = 0
    var closes = 0
    var closeFailure = false
    var losslessReady = false
    var previewReady = false
    var pixelFrame: CaptureStreamFrame? = null
    var receipt: CaptureFreezeReceipt? = null
    private var sequence = 1uL
    override val signals = MutableStateFlow(signalFor(CaptureStreamStage.Live, true))
    private fun signalFor(stage: CaptureStreamStage, available: Boolean) = CaptureStreamSignal(sequence, epoch,
        frame.sourceGeneration, frame.frameId, stage, frame.width, frame.height, 256uL, null, available, "Fixture source")
    fun emit(stage: CaptureStreamStage, available: Boolean = true) { sequence++; signals.value = signalFor(stage, available) }
    override fun signal() = signals.value
    override suspend fun beginDraw(): CaptureStreamDraw? = null
    override suspend fun frozenBackground(): CaptureFrozenBackground? {
        if (!losslessReady && !previewReady) return null
        return DesktopSessionTestPixels(checkNotNull(receipt).draftId, pixelFrame ?: frame, losslessReady, events)
            .also { pixels += it }
    }
    override suspend fun firstContact(draftId: String): CaptureFreezeReceipt {
        contacts++; events += "stream:contact"
        return CaptureFreezeReceipt(draftId, frame.frameId, frame.captureSessionId, frame.sourceGeneration)
            .also { receipt = it; emit(CaptureStreamStage.Freezing) }
    }
    override suspend fun resumeAfterDraftDisposed(): String {
        resumes++; events += "stream:resume"
        frame = frame.copy(frameId = frame.frameId + 1uL)
        receipt = null; losslessReady = false; previewReady = false; pixelFrame = null
        emit(CaptureStreamStage.Live)
        return "resume"
    }
    override suspend fun close() {
        closes++; events += "stream:close"; emit(CaptureStreamStage.Closed, false)
        if (closeFailure) error("stream retirement retained")
    }
}

private class DesktopSessionTestPixels(override val draftId: String, override val frame: CaptureStreamFrame,
    override val lossless: Boolean, private val events: MutableList<String>) : CaptureFrozenBackground {
    override val originalAssetId = if (lossless) "b".repeat(64) else null
    var closes = 0
    override suspend fun rgbaRows(firstRow: UInt, rowCount: UInt): ByteArray = error("The lifecycle owner never reads pixels")
    override suspend fun close() { closes++; events += "pixels:close" }
}

private class DesktopSessionTestDraft(private val number: Int, override val info: CaptureDraftInfo,
    private val events: MutableList<String>) : WorkbenchCaptureDraft {
    override val project = DesktopSessionTestProject("private-$number", info.documentId, events)
    val plan = DesktopSessionTestPlan(info, events)
    var prepares = 0
    var degradedPrepares = 0
    var closes = 0
    var closeFailure = false
    private var closed = false
    override suspend fun prepareSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        original: CaptureFrozenBackground, memoryBudgetBytes: ULong): WorkbenchCaptureDraftSave {
        prepares++; plan.prepare(source, target, metadata, false); return plan
    }
    override suspend fun prepareDegradedSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        preview: CaptureFrozenBackground, memoryBudgetBytes: ULong): WorkbenchCaptureDraftSave {
        prepares++; degradedPrepares++; plan.prepare(source, target, metadata, true); return plan
    }
    override suspend fun close() {
        closes++; events += "draft$number:close"
        if (closeFailure) throw WorkflowFailure(WorkflowFailureKind.Storage)
        if (!closed) { project.close(); closed = true }
    }
}

private class DesktopSessionTestPlan(private val draft: CaptureDraftInfo, private val events: MutableList<String>) : WorkbenchCaptureDraftSave {
    override lateinit var info: CaptureDraftSaveInfo
    var commits = 0
    var closes = 0
    var failCommits = 0
    fun prepare(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata, degraded: Boolean) {
        info = CaptureDraftSaveInfo(draft, source, target, if (degraded) null else "b".repeat(64),
            if (degraded) draft.previewAssetId else "b".repeat(64), degraded, metadata.transactionId, 3u)
    }
    override suspend fun commit(): WorkflowReceipt {
        commits++; events += "plan:commit"
        if (failCommits > 0) { failCommits--; throw WorkflowFailure(WorkflowFailureKind.Storage) }
        return WorkflowReceipt(info.transactionId, ProjectInfo("paired", "Paired", "device", 2uL,
            true, false, 1uL, "saved", listOf("target-document")), commits > 1)
    }
    override suspend fun close() { closes++; events += "plan:close" }
}

private class DesktopSessionTestProject(private val id: String, private val document: String,
    private val events: MutableList<String>) : WorkbenchProject {
    var closes = 0
    override val changes = emptyFlow<ProjectChange>()
    override suspend fun info() = ProjectInfo(id, "Fixture", "device", 1uL, false, false, 0uL, "hash", listOf(document))
    override suspend fun document(documentId: String): DocumentSnapshot = error("Unused")
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("Unused")
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("Unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Unused")
    override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("Unused")
    override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
    override suspend fun close() { closes++; events += "$id:close" }
}

private class DesktopSessionTestLink : ProjectLink {
    var closes = 0
    override val endpoints = emptyList<SessionEndpoint>()
    override val changes = emptyFlow<SessionStatus>()
    override fun status(): SessionStatus = error("Unused")
    override fun sendViewport(documentId: String, corners: List<Point>) = Unit
    override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) = Unit
    override fun streamObject(preview: ObjectPreview) = Unit
    override fun streamNewObject(preview: NewObjectPreview) = Unit
    override suspend fun finishPreview(gestureId: String, cancel: Boolean) = Unit
    override suspend fun peerPreviews(documentId: String): PeerPreviews = error("Unused")
    override suspend fun close() { closes++ }
}
