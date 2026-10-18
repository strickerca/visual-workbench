package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test

class CaptureAnnotationSessionTest {
    @Test fun freezeCreatesOnlyPrivateDraftAndOriginalUpgradeKeepsThatIdentity():Unit=runBlocking {
        val f=Fixture();val project=f.owner.freeze();assertSame(f.draft.project,project);assertEquals(0,f.target.closes)
        val frame=f.owner.state.value.draft;f.receiver.background=f.original;f.owner.refreshOriginal()
        assertTrue(f.owner.state.value.lossless);assertEquals(frame,f.owner.state.value.draft);assertEquals(0,f.original.closes);f.owner.close()
    }
    @Test fun degradedChoiceIsRequiredAndBorrowedPixelsCloseAfterNativePreparation():Unit=runBlocking {
        val f=Fixture();f.owner.freeze();f.receiver.background=f.preview
        assertFails { f.owner.save(true) };assertEquals(0,f.draft.prepares)
        f.receiver.stage=CaptureStreamStage.FrozenDegraded;val receipt=f.owner.save(true)
        assertEquals(receipt,f.owner.state.value.saved);assertTrue(f.owner.state.value.degradedSaved);assertEquals(1,f.preview.closes)
        assertEquals(1,f.draft.degradedPrepares);assertNull(f.draft.plan.info.originalAssetId);f.owner.close();assertEquals(0,f.target.closes)
    }
    @Test fun mismatchedDegradedBackgroundRefusesAndClosesExceptionalHandle():Unit=runBlocking {
        val f=Fixture();f.owner.freeze();f.receiver.stage=CaptureStreamStage.FrozenDegraded
        val wrong=Pixels(false,f.frame.copy(frameId=2uL));f.receiver.background=wrong
        assertFails { f.owner.save(true) };assertEquals(1,wrong.closes);assertEquals(0,f.draft.prepares);assertFalse(f.owner.state.value.saving);f.owner.close()
    }
    @Test fun unknownCommitRetainsSamePlanForDurableRetryAfterDisconnection():Unit=runBlocking {
        val f=Fixture();f.owner.freeze();f.receiver.background=f.original;f.owner.refreshOriginal();f.draft.plan.failCommit=true
        assertFails { f.owner.save() };assertTrue(f.owner.state.value.saveRetry);assertEquals(1,f.draft.prepares)
        f.attached=false;f.receiver.stage=CaptureStreamStage.Closed;val receipt=f.owner.save()
        assertEquals(2,f.draft.plan.commits);assertEquals(1,f.draft.prepares);assertEquals(receipt,f.owner.save());assertEquals(2,f.draft.plan.commits);f.owner.close()
    }
    @Test fun failedDraftCloseRetainsSealedOwnerAndRetryJoinsBeforeResume():Unit=runBlocking {
        val f=Fixture();f.owner.freeze();f.draft.failClose=true
        assertFails { f.owner.resumeAfterEditorDisposed() };assertEquals(0,f.receiver.resumes);assertNotNull(f.owner.state.value.failure)
        f.owner.resumeAfterEditorDisposed();assertEquals(2,f.draft.closes);assertEquals(1,f.receiver.resumes);assertNull(f.owner.state.value.draft);f.owner.close()
    }
    @Test fun resumeCannotPassActualDraftRetirementFence():Unit=runBlocking {
        val f=Fixture();f.owner.freeze();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        f.draft.closeHook={entered.complete(Unit);release.await()}
        val resume=async(start=CoroutineStart.UNDISPATCHED){f.owner.resumeAfterEditorDisposed()}
        try { withTimeout(3000){entered.await()};assertFalse(resume.isCompleted);assertEquals(0,f.receiver.resumes)
            release.complete(Unit);withTimeout(3000){resume.await()};assertEquals(1,f.receiver.resumes)
        } finally {release.complete(Unit);withContext(NonCancellable){resume.await();f.owner.close()}}
    }
    @Test fun childEditorFailureSealsFreezeAndSaveUntilExplicitDiscard():Unit=runBlocking {
        val f=Fixture();f.owner.freeze();f.owner.editorFailed();assertFails{f.owner.freeze()};assertFails{f.owner.save(true)}
        assertEquals(0,f.draft.prepares);f.owner.resumeAfterEditorDisposed();assertFalse(f.owner.state.value.editorFailed);f.owner.close()
    }
    @Test fun wrongOriginalNeverUpgradesReadyStateAndItsOwnerRetires():Unit=runBlocking {
        val f=Fixture();f.owner.freeze();val wrong=Pixels(true,f.frame.copy(sourceGeneration=2uL));f.receiver.background=wrong
        assertFails{f.owner.refreshOriginal()};assertEquals(1,wrong.closes);assertFalse(f.owner.state.value.lossless);f.owner.close()
    }
    @Test fun frameAgeUsesPeerMinusLocalClockOffsetAndNeverClaimsUnsynchronizedAge(){
        assertNull(captureFrameAgeMs(1000,2000,null));assertNull(captureFrameAgeMs(1000,2000,Double.NaN))
        assertEquals(750.0,checkNotNull(captureFrameAgeMs(1000,2000,-250.0)),0.0)
        assertEquals(0.0,checkNotNull(captureFrameAgeMs(1000,100,0.0)),0.0)
    }
    private suspend fun assertFails(body:suspend()->Unit){try{body();fail("unsafe transition accepted")}catch(_:WorkflowFailure){}catch(_:IllegalArgumentException){}}
    private class Fixture {
        val frame=CaptureStreamFrame("session",1uL,1uL,8u,8u,0,0,1.0,1u,10,1000,"windows")
        val target=Project("paired");val preview=Pixels(false,frame);val original=Pixels(true,frame);val receiver=Receiver(frame)
        val draft=Draft(frame);var attached=true;private var next=0
        val owner=CaptureAnnotationSession(target,"target-document",Link(),receiver,
            {listOf("draft","document","layer","request")[next++.coerceAtMost(3)]},
            {WorkflowMetadata("transaction","device",1uL,2000)},{attached},{_,_,_,_->draft})
    }
    private class Pixels(override val lossless:Boolean,override val frame:CaptureStreamFrame):CaptureFrozenBackground {
        override val draftId="draft";override val originalAssetId=if(lossless)"b".repeat(64)else null;var closes=0
        override suspend fun rgbaRows(firstRow:UInt,rowCount:UInt)=ByteArray((frame.width*rowCount*4u).toInt())
        override suspend fun close(){closes++}
    }
    private class Receiver(val frame:CaptureStreamFrame):WorkbenchCaptureStream {
        var stage=CaptureStreamStage.Live;var background:CaptureFrozenBackground?=null;var resumes=0;var closes=0
        override val signals=emptyFlow<CaptureStreamSignal>()
        override fun signal()=CaptureStreamSignal(1uL,1uL,1uL,1uL,stage,8u,8u,256uL,null)
        override suspend fun beginDraw():CaptureStreamDraw?=null
        override suspend fun frozenBackground()=background
        override suspend fun firstContact(draftId:String):CaptureFreezeReceipt {stage=CaptureStreamStage.Freezing;return CaptureFreezeReceipt(draftId,frame.frameId,frame.captureSessionId,frame.sourceGeneration)}
        override suspend fun resumeAfterDraftDisposed():String {resumes++;stage=CaptureStreamStage.Live;return "draft"}
        override suspend fun close(){closes++;stage=CaptureStreamStage.Closed}
    }
    private class Draft(frame:CaptureStreamFrame):WorkbenchCaptureDraft {
        override val info=CaptureDraftInfo("draft","document",frame,"a".repeat(64),1uL);override val project=Project("draft")
        val plan=Plan(info);var prepares=0;var degradedPrepares=0;var closes=0;var failClose=false;var closeHook:suspend()->Unit={}
        override suspend fun prepareSave(source:WorkflowBinding,target:WorkflowBinding,metadata:WorkflowMetadata,original:CaptureFrozenBackground,memoryBudgetBytes:ULong):WorkbenchCaptureDraftSave {prepares++;plan.describe(source,target,metadata,false);return plan}
        override suspend fun prepareDegradedSave(source:WorkflowBinding,target:WorkflowBinding,metadata:WorkflowMetadata,preview:CaptureFrozenBackground,memoryBudgetBytes:ULong):WorkbenchCaptureDraftSave {prepares++;degradedPrepares++;plan.describe(source,target,metadata,true);return plan}
        override suspend fun close(){closes++;closeHook();if(failClose){failClose=false;throw WorkflowFailure(WorkflowFailureKind.Storage)}}
    }
    private class Plan(private val draft:CaptureDraftInfo):WorkbenchCaptureDraftSave {
        override lateinit var info:CaptureDraftSaveInfo;var commits=0;var failCommit=false
        fun describe(source:WorkflowBinding,target:WorkflowBinding,metadata:WorkflowMetadata,degraded:Boolean){info=CaptureDraftSaveInfo(draft,source,target,if(degraded)null else "b".repeat(64),if(degraded)draft.previewAssetId else "b".repeat(64),degraded,metadata.transactionId,3u)}
        override suspend fun commit():WorkflowReceipt {commits++;if(failCommit){failCommit=false;throw WorkflowFailure(WorkflowFailureKind.Storage)};return WorkflowReceipt("transaction",Project("paired").info(),commits>1)}
        override suspend fun close()=Unit
    }
    private class Project(private val id:String):WorkbenchProject {
        var closes=0;override val changes=emptyFlow<ProjectChange>()
        override suspend fun info()=ProjectInfo(id,"project","device",1uL,false,false,0uL,"a".repeat(64),listOf("document"))
        override suspend fun document(documentId:String):DocumentSnapshot=error("unused")
        override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("unused")
        override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("unused")
        override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("unused")
        override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("unused")
        override suspend fun render(documentId:String,rectangle:Rect?):RenderList=error("unused")
        override suspend fun export(options:ExportOptions):ExportResult=error("unused")
        override suspend fun close(){closes++}
    }
    private class Link:ProjectLink {
        override val endpoints=emptyList<SessionEndpoint>();override val changes=emptyFlow<SessionStatus>()
        override fun status():SessionStatus=error("unused")
        override fun sendViewport(documentId:String,corners:List<Point>)=Unit
        override fun streamStroke(stroke:WorkbenchStroke,batch:SampleBatch,firstSample:UInt)=Unit
        override fun streamObject(preview:ObjectPreview)=Unit
        override fun streamNewObject(preview:NewObjectPreview)=Unit
        override suspend fun finishPreview(gestureId:String,cancel:Boolean)=Unit
        override suspend fun peerPreviews(documentId:String):PeerPreviews=error("unused")
        override suspend fun close()=Unit
    }
}
