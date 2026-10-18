@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import com.visualworkbench.android.editor.*

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test

/** App-controller tests stop at the actual shared interface. They neither
 * emulate the provider/proof math nor acquire credentials or native handles. */
class AiEditorOwnershipInstrumentedTest {
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
        assertEquals(driver.saved.last().resultId, editor.state.value.receipt?.resultId)
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
        assertEquals(driver.saved.last().resultId, editor.state.value.receipt?.resultId)
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
    @Test fun gridRetainsIndependentExplicitSendOwnersAndExactSelection() = fixture {
        prepared(); editor.send("request",7u,false)?.join()
        until { editor.state.value.entries.single().preview != null }
        val original = editor.state.value.candidate
        assertTrue(editor.canPrepare()); assertTrue(editor.needsDecision())
        editor.prepare(AiDraftOptions("another option",null))?.join()
        assertEquals(2,editor.state.value.entries.size)
        assertEquals(1,driver.sends); assertEquals(0,drivers[1].sends); assertEquals(0,driver.closes)
        assertNull(editor.state.value.candidate)
        editor.send("request",7u,false)?.join(); assertEquals(0,drivers[1].sends)
        editor.send("request-2",7u,false)?.join()
        until { editor.state.value.entries.all { it.preview != null } }
        editor.selectCandidate("request")?.join()
        assertEquals(original,editor.state.value.candidate)
        assertEquals("request",editor.state.value.review?.requestId)
        assertEquals(listOf(1,1),drivers.map { it.sends })
        assertEquals(listOf("request","request-2"),editor.state.value.entries.map { it.review.requestId })
        assertEquals(listOf("candidate","candidate-request-2"),editor.state.value.entries.map { it.preview?.candidateId })
    }
    @Test fun gridBoundRefusesFifthBeforeCreatingAnotherOwnerAndDiscardFreesOneSlot() = fixture {
        prepared()
        repeat(3) { editor.prepare(AiDraftOptions("",null))?.join() }
        assertEquals(AI_CANDIDATE_LIMIT,editor.state.value.entries.size)
        assertFalse(editor.canPrepare()); assertNull(editor.prepare(AiDraftOptions("",null)))
        assertEquals(AI_CANDIDATE_LIMIT,createdDrivers)
        editor.discard()?.join()
        assertEquals(3,editor.state.value.entries.size); assertTrue(editor.canPrepare())
        assertEquals(1,drivers.last().closes); assertEquals(0,driver.closes)
    }
    @Test fun gridNeverReplacesPaidOwnerWhenNextPreparationFailsOrSourceProofChanges() = fixture {
        prepared(); editor.send("request",7u,false)?.join()
        val candidate = editor.state.value.candidate
        drivers += Driver("request-2").apply { sourceSha = "z".repeat(64) }
        editor.prepare(AiDraftOptions("",null))?.join()
        assertEquals(1,editor.state.value.entries.size); assertEquals(candidate,editor.state.value.candidate)
        assertEquals(0,driver.closes); assertEquals(1,drivers[1].closes); assertEquals(1,driver.sends)
    }
    @Test fun identicalPixelCandidateIdsRemainDistinctRequestOwners() = fixture {
        prepared(); editor.send("request",7u,false)?.join()
        drivers += Driver("request-2").apply { candidateId = "candidate" }
        editor.prepare(AiDraftOptions("",null))?.join(); editor.send("request-2",7u,false)?.join()
        assertEquals(2,editor.state.value.entries.size)
        editor.selectCandidate("request")?.join(); editor.save()?.join()
        assertEquals(1,driver.saved.size); assertTrue(drivers[1].saved.isEmpty())
        editor.selectCandidate("request-2")?.join(); assertNull(editor.state.value.receipt)
        assertTrue(editor.needsDecision())
        editor.selectCandidate("request")?.join()
        assertEquals(driver.saved.single().resultId,editor.state.value.receipt?.resultId)
    }
    @Test fun partialMaskAndExactSaveRetryStayWithTheirSelectedRequest() = fixture {
        prepared(); editor.send("request",7u,false)?.join()
        editor.prepare(AiDraftOptions("",null))?.join(); editor.send("request-2",7u,false)?.join()
        editor.selectCandidate("request")?.join(); until { editor.state.value.frame != null }
        editor.brush(true); until { editor.state.value.frame != null }
        assertTrue(editor.contactDown(Point(.2,.2))); editor.contactUp(); idle()
        val partial = editor.state.value.candidate
        assertTrue(partial!!.partial); assertEquals(1,driver.brushes); assertEquals(0,drivers[1].brushes)
        driver.failSaveOnce = true; editor.save()?.join(); assertTrue(editor.state.value.retrySave)
        editor.selectCandidate("request-2")?.join(); assertFalse(editor.state.value.retrySave); assertFalse(editor.state.value.candidate!!.partial)
        editor.selectCandidate("request")?.join(); assertTrue(editor.state.value.retrySave); assertEquals(partial,editor.state.value.candidate)
        assertFalse(editor.state.value.clearNext)
        editor.save()?.join(); assertEquals(driver.saved[0],driver.saved[1]); assertNotNull(editor.state.value.receipt)
        assertTrue(drivers[1].saved.isEmpty())
    }
    @Test fun staleGridKeepsComparisonsButCannotSendApplyOrAcceptAndCannotPrepareNewGroup() = fixture {
        prepared(); editor.send("request",7u,false)?.join()
        editor.prepare(AiDraftOptions("",null))?.join(); editor.send("request-2",7u,false)?.join()
        attachment = attachment.copy(document=snapshot(2u)); editor.changed()
        assertFalse(editor.canPrepare()); assertTrue(editor.state.value.entries.all { it.stale })
        editor.selectCandidate("request")?.join(); until { editor.state.value.frame != null }
        editor.save()?.join(); assertTrue(driver.saved.isEmpty())
        editor.brush(true); until { editor.state.value.frame != null }; assertFalse(editor.contactDown(Point(.2,.2)))
        editor.send("request",7u,false)?.join(); assertEquals(1,driver.sends)
        assertEquals(2,editor.state.value.entries.size)
    }
    @Test fun switchingGridEntryWaitsForItsCancelledPixelProducer() = fixture {
        prepared(); editor.send("request",7u,false)?.join()
        editor.prepare(AiDraftOptions("",null))?.join(); editor.send("request-2",7u,false)?.join()
        until { editor.state.value.entries.all { it.preview != null } }
        val gate = hold(); drivers[1].pixelGate = gate
        val reads = drivers[1].reads; editor.zoom(1.25); until { drivers[1].reads > reads }
        val selecting = editor.selectCandidate("request")!!; yield()
        assertFalse(selecting.isCompleted); assertEquals("request-2",editor.state.value.review?.requestId)
        gate.complete(Unit); selecting.join()
        until { editor.state.value.frame != null }
        assertEquals("request",editor.state.value.review?.requestId); assertEquals("candidate",editor.state.value.frame?.candidateId)
        assertEquals(listOf(1,1),drivers.map { it.sends })
    }
    @Test fun refusedOutsideProofCannotEnterGridOrBeAppliedAndRecoveryDoesNotResend() = fixture {
        prepared()
        driver.candidateOverride=AiCandidateInfo("request","bad",attachment.binding,2u,2u,8u,false,AiSettlement.UsagePriced,7u,
            proof.copy(changedOutside=1u))
        editor.send("request",7u,false)?.join()
        assertNull(editor.state.value.candidate); assertNull(editor.state.value.entries.single().candidate)
        assertTrue(editor.needsDecision()); assertFalse(editor.canPrepare())
        driver.candidateOverride=null; editor.recoverCandidate()?.join()
        assertEquals("candidate",editor.state.value.candidate?.candidateId); assertEquals(1,driver.sends)
    }
    @Test fun failedDiscardRetainsPaidOwnerAndRetryOnlyClosesSelectedRequest() = fixture {
        prepared(); editor.send("request",7u,false)?.join()
        editor.prepare(AiDraftOptions("",null))?.join(); editor.send("request-2",7u,false)?.join()
        drivers[1].closeFailures=1; editor.discard()?.join()
        assertEquals(2,editor.state.value.entries.size); assertEquals("request-2",editor.state.value.review?.requestId)
        assertTrue(editor.needsDecision()); assertEquals(0,driver.closes)
        editor.discard()?.join()
        assertEquals(1,editor.state.value.entries.size); assertEquals("request",editor.state.value.review?.requestId)
        assertEquals(2,drivers[1].closes); assertEquals(0,driver.closes)
    }
    @Test fun detachClosesEveryGridOwnerButRetainsFailedCloseForJoinedRetry() = fixture {
        prepared(); editor.send("request",7u,false)?.join()
        editor.prepare(AiDraftOptions("",null))?.join(); editor.send("request-2",7u,false)?.join()
        driver.closeFailures=1
        try { editor.detach(); fail("Close failure must be reported") } catch (error:AiFailure) { assertEquals(AiFailureKind.Storage,error.kind) }
        assertTrue(editor.needsDecision()); assertEquals(1,editor.state.value.entries.size)
        assertEquals("request",editor.state.value.review?.requestId); assertEquals(1,drivers[1].closes)
        editor.detach(); assertEquals(2,driver.closes); assertTrue(editor.state.value.entries.isEmpty())
    }
    @Test fun unsupportedProviderReadinessCannotConsumeAnAttemptEvenThroughDirectControllerCall() = fixture {
        prepared(); serviceApi.providerReady=false; settings()
        editor.send("request",7u,false)?.join()
        assertEquals(0,driver.sends); assertFalse(editor.state.value.attempted)
        assertFalse(editor.needsDecision())
        serviceApi.providerReady=true; serviceApi.configured=false; settings()
        editor.send("request",7u,false)?.join(); assertEquals(0,driver.sends); assertFalse(editor.state.value.attempted)
        serviceApi.configured=true; settings(); editor.send("request",7u,false)?.join()
        assertEquals(1,driver.sends)
    }
    @Test fun eachGridReviewKeepsItsExactConfigurationAndOnlyCurrentConfigurationCanSend() = fixture {
        prepared(); serviceApi.fingerprint="second-policy"; settings()
        editor.prepare(AiDraftOptions("",null))?.join()
        assertEquals(listOf("policy","second-policy"),editor.state.value.entries.map { it.review.configurationFingerprint })
        editor.selectCandidate("request")?.join(); editor.send("request",7u,false)?.join()
        assertEquals(0,driver.sends); assertFalse(editor.state.value.attempted)
        editor.selectCandidate("request-2")?.join(); editor.send("request-2",7u,false)?.join()
        assertEquals(1,drivers[1].sends)
    }
    @Test fun mismatchedSaveReceiptCannotReplaceTheExactRetainedSaveTransaction() = fixture {
        prepared(); editor.send("request",7u,false)?.join(); driver.wrongReceiptOnce=true
        editor.save()?.join(); assertNull(editor.state.value.receipt); assertTrue(editor.state.value.retrySave)
        editor.save()?.join()
        assertEquals(driver.saved[0],driver.saved[1]); assertEquals(driver.saved.last().resultId,editor.state.value.receipt?.resultId)
        assertEquals(1,driver.sends)
    }
    @Test fun publishedGridPreviewFollowsRendererLifetimeWhenItsOwnerIsDiscarded() = fixture {
        prepared(); editor.send("request",7u,false)?.join()
        until { editor.state.value.entries.single().preview != null && editor.state.value.frame != null }
        val publishedPreview = editor.state.value.entries.single().preview!!.image
        editor.discard()?.join()
        assertTrue(editor.state.value.entries.isEmpty())
        assertFalse(abandoned.contains(publishedPreview))
    }
    @Test fun cancelledUnpublishedPreviewIsAbandonedBeforeDiscardReturns() = fixture {
        prepared(); val gate=hold();val entered=CompletableDeferred<Unit>();imageGate=gate;imageEntered=entered
        editor.send("request",7u,false)?.join(); withTimeout(3000){entered.await()}
        val preview = made.last();val discarding=editor.discard()!!;yield()
        assertFalse(discarding.isCompleted)
        gate.complete(Unit);discarding.join()
        assertTrue(abandoned.contains(preview));assertTrue(editor.state.value.entries.isEmpty())
    }
    @Test fun largeCandidateGridPreviewRemainsBoundedWhenFullDetailIsRefused() = fixture {
        attachment = attachment.copy(document=snapshot(1u,width=8192u,height=8192u))
        api.width=8192u;api.height=8192u;driver.width=8192u;driver.height=8192u
        prepared();editor.viewport(8192.0,8192.0);editor.send("request",7u,false)?.join()
        until { editor.state.value.entries.single().preview != null && editor.state.value.message != null && !editor.state.value.loadingPixels }
        assertEquals(AiRegion(3968u,3968u,256u,256u),editor.state.value.entries.single().preview?.region)
        assertEquals(listOf(AiRegion(3968u,3968u,256u,256u)),driver.regions)
        assertNull(editor.state.value.frame);assertEquals(1,driver.sends)
        assertTrue(editor.state.value.message!!.contains("64 MiB"))
    }
    @Test fun reviewedGlobalInstructionStaysWithEachGridEntry() = fixture {
        prepared();editor.prepare(AiDraftOptions("second immutable instruction",null))?.join()
        assertEquals(listOf("","second immutable instruction"),editor.state.value.entries.map { it.instruction })
        editor.selectCandidate("request")?.join()
        assertEquals("",editor.state.value.entries.first { it.review.requestId == editor.state.value.review?.requestId }.instruction)
        assertEquals(listOf(0,0),drivers.map { it.sends })
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
private fun snapshot(seq: ULong, results: Boolean = true, width: UInt = 2u, height: UInt = 2u): DocumentSnapshot {
    val info = ProjectInfo("project", "fixture", "device", seq + 1u, true, false, seq, seq.toString().padStart(64, '0'), listOf("document"))
    val items = if (!results) emptyList() else listOf(RenderItem("object", "layer", 1.0, "normal", Rect(0.0,0.0,2.0,2.0),
        Contours(longArrayOf(),longArrayOf(),uintArrayOf()), Transform(), ObjectStyle(0xffffffffu,0.0), Shape.Result("c".repeat(64), "saved"), false))
    return DocumentSnapshot("document", "fixture", width, height, 8u, listOf(LayerInfo("layer","Result",true,false,1.0,"normal")), RenderList(info, items))
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
    var fingerprint = "policy"; var providerReady = true; var configured = true
    override suspend fun configuration() = AiConfiguration("{}",fingerprint,"2026-10-01","2026-10-31","fixture","fixture",1_000_000u)
    override suspend fun configure(json: String, expectedFingerprint: String) = configuration()
    override suspend fun configureDailyBudget(expectedFingerprint: String, dailySoftBudgetMicrousd: ULong) = configuration()
    override suspend fun readiness() = AiReadiness(configured,true,true,0u,1_000_000u,providerReady)
    override suspend fun saveWindowsKey(bytes: ByteArray) = error("No credentials in fixture")
    override suspend fun removeWindowsKey() = error("No credentials in fixture")
    override suspend fun close() = Unit
}
private class Api : WorkbenchAiResults {
    var width = 2u; var height = 2u
    var contexts = 0; var reads = 0; var refuse = false; var pixelGate: CompletableDeferred<Unit>? = null
    override suspend fun context(binding: WorkflowBinding, memoryBudgetBytes: ULong): AiContext {
        contexts++; return AiContext(binding,"s".repeat(64),width,height,8u,emptyList(),emptyList())
    }
    override suspend fun resultPixels(binding: WorkflowBinding, resultId: String, region: AiRegion, memoryBudgetBytes: ULong): AiResultPixels {
        reads++; val gate = pixelGate; pixelGate = null
        if (gate != null) withContext(NonCancellable) { gate.await() }
        if (refuse) throw AiFailure(AiFailureKind.Proof)
        return AiResultPixels(binding,resultId,"s".repeat(64),"c".repeat(64),region,2u,2u,ByteArray((region.width*region.height*4u).toInt()))
    }
    override suspend fun resultStatus(binding: WorkflowBinding, resultId: String, accepted: Boolean, metadata: WorkflowMetadata): WorkflowReceipt = error("No status in fixture")
}
private class Driver(val requestId: String = "request") : AiDriver {
    var prepares = 0; var sends = 0; var closes = 0; var brushes = 0; var failSaveOnce = false; var failPrepare = false
    var prepareGate: CompletableDeferred<Unit>? = null; var sendGate: CompletableDeferred<Unit>? = null
    lateinit var binding: WorkflowBinding
    val saved = mutableListOf<AiSaveOptions>()
    var candidateId = if (requestId == "request") "candidate" else "candidate-$requestId"
    var recoveries = 0; var closeFailures = 0; var partial = false
    var sourceSha = "a".repeat(64); var maskSha = "b".repeat(64); var width = 2u; var height = 2u
    val regions = mutableListOf<AiRegion>()
    var candidateOverride: AiCandidateInfo? = null; var wrongReceiptOnce = false
    var pixelGate: CompletableDeferred<Unit>? = null; var reads = 0
    private fun candidate() = candidateOverride ?: AiCandidateInfo(requestId,candidateId,binding,width,height,8u,partial,AiSettlement.UsagePriced,7u,
        if (partial) proof.copy(changedUnaccepted=0u,unacceptedSha256Before="a".repeat(64),unacceptedSha256After="a".repeat(64)) else proof)
    override suspend fun recover():AiCandidateInfo{recoveries++;return candidate()}
    override suspend fun prepare(options: AiPrepareOptions): AiReview {
        prepares++; binding = options.binding
        if (failPrepare) throw AiFailure(AiFailureKind.Invalid)
        prepareGate?.let { withContext(NonCancellable) { it.await() } }
        return AiReview(requestId,binding,"s".repeat(64),sourceSha,maskSha,options.configurationFingerprint,"fixture","fixture",width,height,8u,
            width,height,0,0,width,height,8u,7u,"fixture","2026-10-31","2026-10-31",false)
    }
    override suspend fun send(request: String, estimate: ULong, acknowledge: Boolean): AiCandidateInfo {
        sends++; sendGate?.let { withContext(NonCancellable) { it.await() } }; currentCoroutineContext().ensureActive(); return candidate()
    }
    override suspend fun pixels(mode: AiCompareMode, region: AiRegion): AiPixels {
        reads++; regions += region; val gate = pixelGate; pixelGate = null
        gate?.let { withContext(NonCancellable) { it.await() } }
        return AiPixels(candidateId,region,if(mode==AiCompareMode.Split)width*2u else width,height,ByteArray((region.width*region.height*4u).toInt()))
    }
    override suspend fun brush(value: AiAcceptanceBrush): AiCandidateInfo {
        check(value.expectedCandidateId == candidateId); brushes++; candidateId = value.nextCandidateId; partial = true; return candidate()
    }
    override suspend fun save(value: AiSaveOptions): AiResultReceipt {
        saved += value
        if (failSaveOnce) { failSaveOnce=false; throw AiFailure(AiFailureKind.Storage) }
        val resultId = if (wrongReceiptOnce) { wrongReceiptOnce=false; "wrong-result" } else value.resultId
        return AiResultReceipt(value.metadata.transactionId,resultId,value.objectId,value.layerId,"o".repeat(64),"c".repeat(64),null,snapshot(2u).render.revision,candidate().proof)
    }
    override suspend fun close() { closes++; if (closeFailures > 0) { closeFailures--; throw AiFailure(AiFailureKind.Storage) } }
}
private class Fixture(val scope: CoroutineScope) : CoroutineScope by scope {
    val gates = mutableListOf<CompletableDeferred<Unit>>()
    fun hold() = CompletableDeferred<Unit>().also { gates += it }
    val core = Core(); val project = Project(); val api = Api()
    val drivers = mutableListOf(Driver()); val driver get() = drivers.first()
    var createdDrivers = 0
    var attachment = AiAttachment(project,snapshot(1u),1)
    var serviceCreates = 0; var failRefresh = false; private var bitmaps = 0
    val abandoned = mutableListOf<Int>(); val made = mutableListOf<Int>()
    var imageGate: CompletableDeferred<Unit>? = null; var imageEntered: CompletableDeferred<Unit>? = null
    val serviceApi = Service()
    val service = AiServiceController(scope,"unused") { _, _ -> serviceCreates++; serviceApi }
    val result = AiResultController(scope,core,{attachment},{Camera(Point(1.0,1.0),1.0,0.0,2.0,2.0)},
        image = { ++bitmaps }, abandonImage = { abandoned += it }, results = { api })
    val editor = AiEditorController(scope,core,service,{attachment},Mutex(),
        metadata = { WorkflowMetadata("txn","device",3u,0) },
        refreshAfterMutation = { if(failRefresh) throw AiFailure(AiFailureKind.Storage) },
        image = {
            val bitmap = ++bitmaps; made += bitmap
            val gate = imageGate; imageGate = null
            imageEntered?.complete(Unit)
            gate?.let { withContext(NonCancellable) { it.await() } }
            bitmap
        }, abandonImage = { abandoned += it }, results = { api }, driverFactory = { _,_,_ ->
            if (createdDrivers == drivers.size) drivers += Driver("request-${createdDrivers+1}")
            drivers[createdDrivers++]
        })
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
