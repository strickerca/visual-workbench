@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test

/** App-controller tests stop at the actual shared interface. They neither
 * emulate the provider/proof math nor acquire credentials or native handles. */
class AiEditorOwnershipTest {
    @Test fun cancelledSendRetainsOwnershipAcrossHideReopenAndDetach() = fixture {
        prepared(); val gate = hold(); driver.sendGate = gate
        val sending = editor.send("request", 7u, false)!!
        until { driver.sends == 1 }; val contexts = api.contexts
        editor.cancel(); assertFalse(sending.isActive); assertFalse(sending.isCompleted)
        editor.hide(); editor.open(); assertNull(editor.reloadContext())
        assertEquals(contexts, api.contexts); assertEquals(AiStage.Sending, editor.state.value.busy)
        val closing = launch { editor.detach() }; yield()
        assertFalse(closing.isCompleted); assertEquals(0, driver.closes)
        gate.complete(Unit); sending.join(); closing.join()
        assertEquals(1, driver.closes); assertNull(editor.state.value.review)
    }
    @Test fun cancelledPreparationCannotLoseItsLateHandleToAReopen() = fixture {
        settings(); editor.open(); idle(); val gate = hold(); driver.prepareGate = gate
        val preparing = editor.prepare(AiDraftOptions("", null))!!
        until { driver.prepares == 1 }; val contexts = api.contexts
        editor.cancel(); editor.hide(); editor.open()
        assertNull(editor.reloadContext()); assertEquals(contexts, api.contexts)
        assertFalse(preparing.isCompleted)
        val closing = launch { editor.detach() }; yield(); assertFalse(closing.isCompleted)
        gate.complete(Unit); preparing.join(); closing.join()
        assertEquals(1, driver.closes); assertNull(editor.state.value.review)
    }
    @Test fun preparingAnotherRequestCannotDiscardPaidCandidateOrImmutableSaveRetry() = fixture {
        prepared(); editor.send("request", 7u, false)?.join()
        driver.failSaveOnce = true; editor.save()?.join()
        val candidate = editor.state.value.candidate; val review = editor.state.value.review
        assertTrue(editor.state.value.retrySave); driver.failPrepare = true
        assertNull(editor.prepare(AiDraftOptions("replacement that would fail", null)))
        assertEquals(candidate, editor.state.value.candidate); assertEquals(review, editor.state.value.review)
        assertTrue(editor.state.value.retrySave); assertEquals(1, driver.prepares); assertEquals(0, driver.closes)
        editor.save()?.join()
        assertEquals(2, driver.saved.size); assertEquals(driver.saved[0], driver.saved[1])
        assertEquals("saved", editor.state.value.receipt?.resultId)
    }
    @Test fun preparingAfterAnUncertainAttemptKeepsTheRecoverablePaidRequest() = fixture {
        prepared(); val gate = hold(); driver.sendGate = gate
        val sending = editor.send("request", 7u, false)!!
        until { driver.sends == 1 }; editor.cancel(); gate.complete(Unit); sending.join()
        assertTrue(editor.needsDecision()); assertNull(editor.state.value.candidate)
        val review = editor.state.value.review; driver.failPrepare = true
        assertNull(editor.prepare(AiDraftOptions("replacement that would fail", null)))
        assertEquals(review, editor.state.value.review); assertEquals(1, driver.prepares); assertEquals(0, driver.closes)
        driver.candidateId = "completed-before-cancelled-delivery"
        editor.recoverCandidate()?.join()
        assertEquals("completed-before-cancelled-delivery", editor.state.value.candidate?.candidateId)
        assertEquals(1, driver.sends)
    }
    @Test fun ambiguousPartialCandidateCanBeReloadedWithoutAnotherSend() = fixture {
        prepared();editor.send("request",7u,false)?.join()
        driver.candidateId="completed-private-brush"
        editor.recoverCandidate()?.join()
        assertEquals("completed-private-brush",editor.state.value.candidate?.candidateId)
        assertEquals(1,driver.sends);assertEquals(1,driver.recoveries)
    }
    @Test fun hidingPaidReviewKeepsItUntilExplicitSaveOrDiscard() = fixture {
        prepared();editor.send("request",7u,false)?.join()
        editor.hide();assertTrue(editor.needsDecision());assertEquals(0,driver.closes)
        editor.discard()?.join();assertFalse(editor.needsDecision());assertEquals(1,driver.closes)
        assertNull(editor.state.value.candidate);assertEquals(1,driver.sends)
    }
    @Test fun sameDocumentIdsOnANewProjectHandleDoNotAuthorizeTheOldReview() = fixture {
        prepared();attachment=AiAttachment(Project(),snapshot(1u),2);editor.changed()
        editor.send("request",7u,false)?.join();assertEquals(0,driver.sends)
    }
    @Test fun volatilePreviewsCannotIntroduceAResultOrReplaceItsAsset() {
        val doc=snapshot(1u);val item=doc.render.items.single()
        assertTrue(aiAdmitResultPreview(doc,item.copy(transform=Transform(e=1.0))))
        assertFalse(aiAdmitResultPreview(doc,item.copy(objectId="unpublished")))
        assertFalse(aiAdmitResultPreview(doc,item.copy(shape=Shape.Result("d".repeat(64),"result-other"))))
        assertFalse(aiAdmitResultPreview(doc,item.copy(objectId="new-vector",shape=Shape.Rectangle(Rect(0.0,0.0,1.0,1.0)))))
    }
    @Test fun unchangedResultLoadingIsCoalescedAndDoesNotTouchAProvider() = fixture {
        val gate = hold(); api.pixelGate = gate
        result.request(); until { api.reads == 1 }
        repeat(20) { result.request() }; yield()
        assertEquals(1, api.reads)
        assertEquals(0, serviceCreates)
        gate.complete(Unit); until { result.state.value.frame != null }
        assertEquals(setOf("object"), result.state.value.frame!!.images.keys)
    }
    @Test fun oldResultReadCannotReappearAfterAVisibleRevisionChange() = fixture {
        val gate = hold(); api.pixelGate = gate
        result.request(); until { api.reads == 1 }
        attachment = attachment.copy(document = snapshot(2u))
        result.request(); gate.complete(Unit)
        until { result.state.value.frame?.attachment?.binding == attachment.binding }
        assertTrue(abandoned.isNotEmpty())
        assertEquals(2uL, result.state.value.frame!!.attachment.binding.hostSeq)
    }
    @Test fun resultDetachWaitsForNonCancellableProducerThenDropsItsBitmap() = fixture {
        val gate = hold(); api.pixelGate = gate
        result.request(); until { api.reads == 1 }
        val closing = launch { result.detach() }; yield()
        assertFalse(closing.isCompleted)
        gate.complete(Unit); closing.join()
        assertNull(result.state.value.frame)
        assertEquals(listOf(1), abandoned)
    }
    @Test fun unopenedEditorAndOrdinaryNoResultDocumentAreCapabilityLazy() = fixture {
        assertEquals(0, api.contexts); assertEquals(0, serviceCreates)
        attachment = attachment.copy(document = snapshot(1u, results = false))
        result.request(); until { result.state.value.frame != null }
        assertEquals(0, api.contexts); assertEquals(0, serviceCreates)
    }
    @Test fun resultFailureWaitsForExplicitRetryInsteadOfARefreshLoop() = fixture {
        api.refuse = true; result.request(); until { result.state.value.message != null }
        repeat(10) { result.request() }; yield()
        assertEquals(1, api.reads)
        api.refuse = false; result.retry(); until { result.state.value.frame != null }
        assertEquals(2, api.reads)
    }
    @Test fun displayedSendIdentityAndCostMustMatchTheRetainedReview() = fixture {
        prepared()
        editor.send("different-review", 7u, false)?.join()
        editor.send("request", 8u, false)?.join()
        assertEquals(0, driver.sends)
        editor.send("request", 7u, false)?.join()
        assertEquals(1, driver.sends)
        assertEquals("candidate", editor.state.value.candidate?.candidateId)
    }
    @Test fun cancelledSendIsNeverAutomaticallyRepeated() = fixture {
        prepared(); val gate = hold(); driver.sendGate = gate
        val sending = editor.send("request", 7u, false)!!
        until { driver.sends == 1 }; editor.cancel(); gate.complete(Unit); sending.join()
        assertTrue(editor.state.value.attempted)
        editor.send("request", 7u, true)?.join()
        assertEquals(1, driver.sends)
    }
    @Test fun documentMutationBetweenReviewAndSendRefusesBeforeAttempt() = fixture {
        prepared(); attachment = attachment.copy(document = snapshot(2u)); editor.changed()
        editor.send("request", 7u, false)?.join()
        assertEquals(0, driver.sends); assertTrue(editor.state.value.stale)
    }
    @Test fun latePreparedRequestIsClosedBeforeProjectDetachReturns() = fixture {
        settings(); editor.open(); idle(); val gate = hold(); driver.prepareGate = gate
        editor.prepare(AiDraftOptions("", null)); until { driver.prepares == 1 }
        val closing = launch { editor.detach() }; yield()
        assertFalse(closing.isCompleted)
        gate.complete(Unit); closing.join()
        assertEquals(1, driver.closes); assertNull(editor.state.value.review)
    }
    @Test fun acceptedSaveReceiptSurvivesFailedRefreshAndUsesOneImmutableRetry() = fixture {
        prepared(); editor.send("request", 7u, false)?.join()
        driver.failSaveOnce = true
        editor.save()?.join(); assertTrue(editor.state.value.retrySave)
        attachment = attachment.copy(document = snapshot(2u)); editor.changed()
        failRefresh = true
        editor.save()?.join()
        assertEquals(2, driver.saved.size)
        assertEquals(driver.saved[0], driver.saved[1])
        assertEquals("saved", editor.state.value.receipt?.resultId)
        assertFalse(editor.state.value.retrySave)
    }
    @Test fun partialAcceptanceHasOneUpAndCancelledOrMovedViewHasNone() = fixture {
        prepared(); editor.send("request", 7u, false)?.join()
        until { editor.state.value.frame != null }
        editor.brush(true); until { editor.state.value.frame != null }
        assertTrue(editor.contactDown(Point(.2, .2)))
        editor.cancelContact(); editor.contactUp(); yield(); assertEquals(0, driver.brushes)
        assertTrue(editor.contactDown(Point(.2, .2)))
        editor.pan(1.0, 0.0); editor.contactUp(); yield(); assertEquals(0, driver.brushes)
        until { editor.state.value.frame != null }
        // Fixture camera mapping is identity, so this remains an in-image point.
        assertTrue(editor.contactDown(Point(.2, .2)))
        editor.contactUp(); editor.contactUp(); idle(); assertEquals(1, driver.brushes)
    }
    @Test fun estimateInputRequiresExplicitBoundedDatesAndIntegerCounts() {
        assertNull(aiEstimateInput("", "", "", "", "", ""))
        assertEquals(3uL, aiEstimateInput("1", "2", "3", "owner calculation", "2026-10-01", "2026-10-31")!!.imageOutput)
        for (date in listOf("2026-02-30", "2026-10-1", "2026-12-01")) {
            try { aiEstimateInput("1", "2", "3", "fixture", "2026-10-01", date); fail("Expected dated estimate refusal") }
            catch (_: IllegalArgumentException) { }
        }
    }
}

private suspend fun until(check: () -> Boolean) { withTimeout(3_000) { while (!check()) yield() } }
private val proof = AiProofInfo(0u, "a".repeat(64), "a".repeat(64), null, null, null, 0.0, 0.0, 1.0)
private fun snapshot(seq: ULong, results: Boolean = true): DocumentSnapshot {
    val info = ProjectInfo("project", "fixture", "device", seq + 1u, true, false, seq, seq.toString().padStart(64, '0'), listOf("document"))
    val items = if (!results) emptyList() else listOf(RenderItem("object", "layer", 1.0, "normal", Rect(0.0,0.0,2.0,2.0),
        Contours(longArrayOf(),longArrayOf(),uintArrayOf()), Transform(), ObjectStyle(0xffffffffu,0.0), Shape.Result("c".repeat(64), "saved"), false))
    return DocumentSnapshot("document", "fixture", 2u, 2u, 8u, listOf(LayerInfo("layer","Result",true,false,1.0,"normal")), RenderList(info, items))
}
private class Project : WorkbenchProject {
    override val changes = MutableSharedFlow<ProjectChange>()
    override suspend fun info() = snapshot(1u).render.revision
    override suspend fun document(documentId: String) = snapshot(1u)
    override suspend fun close() = Unit
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("No stroke in AI fixture")
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("No image decoder in fixture")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("No ordinary mutation in fixture")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("No undo in fixture")
    override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("No native renderer in fixture")
    override suspend fun export(options: ExportOptions): ExportResult = error("No export in fixture")
}
private class Core : WorkbenchCore {
    private var ids = 0
    override fun newId(unixMs: ULong) = "id-${++ids}"
    override fun newDeviceId() = "device"
    override suspend fun create(options: CreateProject): WorkbenchProject = error("No native creation")
    override suspend fun open(path: String): WorkbenchProject = error("No filesystem")
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("No layout")
    override fun cameraMatrix(camera: Camera, inverse: Boolean) = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>) = points
}
private class Service : WorkbenchAiService {
    override suspend fun configuration() = AiConfiguration("{}","policy","2026-10-01","2026-10-31","fixture","fixture",1_000_000u)
    override suspend fun configure(json: String, expectedFingerprint: String) = configuration()
    override suspend fun configureDailyBudget(expectedFingerprint: String, dailySoftBudgetMicrousd: ULong) = configuration()
    override suspend fun readiness() = AiReadiness(true,true,true,0u,1_000_000u)
    override suspend fun saveWindowsKey(bytes: ByteArray) = error("No credentials in fixture")
    override suspend fun removeWindowsKey() = error("No credentials in fixture")
    override suspend fun close() = Unit
}
private class Api : WorkbenchAiResults {
    var contexts = 0; var reads = 0; var refuse = false; var pixelGate: CompletableDeferred<Unit>? = null
    override suspend fun context(binding: WorkflowBinding, memoryBudgetBytes: ULong): AiContext {
        contexts++; return AiContext(binding,"s".repeat(64),2u,2u,8u,emptyList(),emptyList())
    }
    override suspend fun resultPixels(binding: WorkflowBinding, resultId: String, region: AiRegion, memoryBudgetBytes: ULong): AiResultPixels {
        reads++; val gate = pixelGate; pixelGate = null
        if (gate != null) withContext(NonCancellable) { gate.await() }
        if (refuse) throw AiFailure(AiFailureKind.Proof)
        return AiResultPixels(binding,resultId,"s".repeat(64),"c".repeat(64),region,2u,2u,ByteArray((region.width*region.height*4u).toInt()))
    }
    override suspend fun resultStatus(binding: WorkflowBinding, resultId: String, accepted: Boolean, metadata: WorkflowMetadata): WorkflowReceipt = error("No status in fixture")
}
private class Driver : AiDriver {
    var prepares = 0; var sends = 0; var closes = 0; var brushes = 0; var failSaveOnce = false; var failPrepare = false
    var prepareGate: CompletableDeferred<Unit>? = null; var sendGate: CompletableDeferred<Unit>? = null
    lateinit var binding: WorkflowBinding
    val saved = mutableListOf<AiSaveOptions>()
    var candidateId="candidate";var recoveries=0
    private fun candidate() = AiCandidateInfo("request",candidateId,binding,2u,2u,8u,false,AiSettlement.UsagePriced,7u,proof)
    override suspend fun recover():AiCandidateInfo{recoveries++;return candidate()}
    override suspend fun prepare(options: AiPrepareOptions): AiReview {
        prepares++; binding = options.binding
        if (failPrepare) throw AiFailure(AiFailureKind.Invalid)
        prepareGate?.let { withContext(NonCancellable) { it.await() } }
        return AiReview("request",binding,"s".repeat(64),"a".repeat(64),"b".repeat(64),"policy","fixture","fixture",2u,2u,8u,
            2u,2u,0,0,2u,2u,8u,7u,"fixture","2026-10-31","2026-10-31",false)
    }
    override suspend fun send(request: String, estimate: ULong, acknowledge: Boolean): AiCandidateInfo {
        sends++; sendGate?.let { withContext(NonCancellable) { it.await() } }; currentCoroutineContext().ensureActive(); return candidate()
    }
    override suspend fun pixels(mode: AiCompareMode, region: AiRegion) = AiPixels("candidate",region,if(mode==AiCompareMode.Split)4u else 2u,2u,ByteArray((region.width*region.height*4u).toInt()))
    override suspend fun brush(value: AiAcceptanceBrush): AiCandidateInfo { brushes++; return candidate().copy(candidateId=value.nextCandidateId,partial=true) }
    override suspend fun save(value: AiSaveOptions): AiResultReceipt {
        saved += value
        if (failSaveOnce) { failSaveOnce=false; throw AiFailure(AiFailureKind.Storage) }
        return AiResultReceipt(value.metadata.transactionId,"saved","object","layer","o".repeat(64),"c".repeat(64),null,snapshot(2u).render.revision,proof)
    }
    override suspend fun close() { closes++ }
}
private class Fixture(val scope: CoroutineScope) : CoroutineScope by scope {
    val gates = mutableListOf<CompletableDeferred<Unit>>()
    fun hold() = CompletableDeferred<Unit>().also { gates += it }
    val core = Core(); val project = Project(); val api = Api(); val driver = Driver()
    var attachment = AiAttachment(project,snapshot(1u),1)
    var serviceCreates = 0; var failRefresh = false; private var bitmaps = 0
    val abandoned = mutableListOf<Int>()
    val service = AiServiceController(scope,"unused") { _, _ -> serviceCreates++; Service() }
    val result = AiResultController(scope,core,{attachment},{Camera(Point(1.0,1.0),1.0,0.0,2.0,2.0)},
        image = { ++bitmaps }, abandonImage = { abandoned += it }, results = { api })
    val editor = AiEditorController(scope,core,service,{attachment},Mutex(),
        metadata = { WorkflowMetadata("txn","device",3u,0) },
        refreshAfterMutation = { if(failRefresh) throw AiFailure(AiFailureKind.Storage) },
        image = { ++bitmaps }, abandonImage = { abandoned += it }, results = { api }, driverFactory = { _,_,_ -> driver })
    suspend fun idle() = until { editor.state.value.idle }
    suspend fun settings() { service.activate(); until { !service.state.value.busy } }
    suspend fun prepared() { settings(); editor.open(); idle(); editor.prepare(AiDraftOptions("",null))?.join() }
    suspend fun close() { gates.forEach { it.complete(Unit) }; try { editor.close() } finally { try { result.close() } finally { service.close() } } }
}
private fun fixture(block: suspend Fixture.() -> Unit) = runBlocking {
    val owner = CoroutineScope(coroutineContext + SupervisorJob(coroutineContext[Job]))
    val fixture = Fixture(owner)
    try { withTimeout(10_000) { fixture.block() } }
    finally { withContext(NonCancellable) { try { fixture.close() } finally { owner.cancel() } } }
}
