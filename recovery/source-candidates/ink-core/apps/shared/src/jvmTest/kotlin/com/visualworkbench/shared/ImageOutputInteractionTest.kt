package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test

class ImageOutputInteractionTest {
    @Test fun unusedCapabilityStaysLazyAndExplicitExportLoadsWithoutOpeningPanel()=runBlocking {
        val f=Fixture();try {
            f.publish();repeat(3){yield()};assertEquals(0,f.capabilities)
            assertNull(f.controls.exportSnapshot(f.document()))
            f.controls.requestExportContext();f.idle()
            assertFalse(f.controls.state.value.active);assertNotNull(f.controls.exportSnapshot(f.document()))
            assertEquals(1,f.reads)
        }finally{f.close()}
    }
    @Test fun privateValuesDoNotMutateAndInvalidCropAcquiresNoIdsOrPlan()=runBlocking {
        val f=Fixture();try {
            f.open();val form=f.frame().copy(cropWidth="99999")
            assertTrue(f.controls.edit(form));assertEquals(0,f.prepares);assertEquals(0,f.ids)
            assertFalse(f.controls.apply());assertEquals(0,f.prepares);assertEquals(0,f.ids)
            assertTrue(f.controls.state.value.dirty);assertFalse(f.controls.active(false))
            assertTrue(f.controls.discardFormAndReload());f.idle();assertFalse(f.controls.state.value.dirty)
        }finally{f.close()}
    }
    @Test fun immutableCommandAdmitsOnePlanAndOneCommitDespiteRepeatedClicks()=runBlocking {
        val f=Fixture();try {
            f.open();f.controls.edit(f.frame().copy(outputWidth="32",outputHeight="48"))
            f.gate.lock();assertTrue(f.controls.apply());assertFalse(f.controls.apply())
            assertFalse(f.controls.edit(f.frame().copy(outputWidth="9")))
            f.gate.unlock();f.idle()
            val command=f.commands.single() as ImageOutputCommand.SetFrame
            assertEquals(32u,command.settings.outputWidth);assertEquals(48u,command.settings.outputHeight)
            assertEquals(1,f.prepares);assertEquals(1,f.commits);assertEquals(1,f.closedPlans)
            assertNotNull(f.controls.state.value.lastReceipt)
        }finally{if(f.gate.isLocked)f.gate.unlock();f.close()}
    }
    @Test fun laterOpsClearPixelsBeforeDelayedSnapshotCanPublish()=runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        try {
            f.readGate={entered.complete(Unit);release.await()};f.publish();f.controls.active(true);entered.await()
            f.controls.observe(1uL);release.complete(Unit);repeat(3){yield()}
            assertNull(f.controls.state.value.snapshot);assertNull(f.controls.exportSnapshot(f.document()))
            f.readGate=null;f.info=f.info.copy(stateHash="b".repeat(64));f.publish(1uL);f.idle()
            assertEquals(f.info.stateHash,f.controls.state.value.snapshot!!.binding.stateHash)
        }finally{release.complete(Unit);f.close()}
    }
    @Test fun privateFormRemainsStaleAcrossRevisionUntilExplicitDiscard()=runBlocking {
        val f=Fixture();try {
            f.open();val draft=f.frame().copy(cropWidth="12");f.controls.edit(draft)
            f.info=f.info.copy(stateHash="c".repeat(64));f.controls.observe(1uL);f.publish(1uL);f.idle()
            assertEquals(draft,f.controls.state.value.form);assertTrue(f.controls.state.value.stale)
            assertFalse(f.controls.apply());assertFalse(f.controls.selectTone(null))
            f.controls.discardFormAndReload();f.idle();assertTrue(f.controls.state.value.ready)
        }finally{f.close()}
    }
    @Test fun metadataDelayRechecksRevisionBeforePreparing()=runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        try {
            f.open();f.metadataGate={entered.complete(Unit);release.await()}
            assertTrue(f.controls.apply());entered.await();f.info=f.info.copy(stateHash="d".repeat(64));f.publish(1uL)
            release.complete(Unit);f.idle();assertEquals(0,f.prepares);assertEquals(0,f.commits)
        }finally{release.complete(Unit);f.close()}
    }
    @Test fun cancelledCommitKeepsOwnershipUntilDetachSettles()=runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();var detached:Deferred<Unit>?=null
        try {
            f.open();f.commitGate={withContext(NonCancellable){entered.complete(Unit);release.await()};currentCoroutineContext().ensureActive()}
            assertTrue(f.controls.apply());entered.await();f.controls.cancel()
            assertFalse(f.controls.active(false));f.controls.active(true)
            assertFalse(f.controls.apply());assertTrue(f.controls.state.value.busy)
            detached=async(start=CoroutineStart.UNDISPATCHED){f.controls.detach()}
            assertFalse(detached.isCompleted);assertEquals(0,f.closedPlans)
            release.complete(Unit);withTimeout(3000){detached.await()}
            assertEquals(1,f.closedPlans);assertFalse(f.controls.state.value.busy)
        }finally{release.complete(Unit);withContext(NonCancellable){detached?.join();f.close()}}
    }
    @Test fun uncertainEditRetainsExactRetryEvenAfterVisibleRevisionChanges()=runBlocking {
        val f=Fixture();try {
            f.open();f.failCommit=true;assertTrue(f.controls.apply());f.idle()
            val id=f.controls.state.value.retry!!.transactionId
            assertFalse(f.controls.state.value.mayLeave);assertFalse(f.controls.apply())
            f.info=f.info.copy(stateHash="e".repeat(64));f.publish(1uL);f.idle()
            f.failCommit=false;assertTrue(f.controls.retry());f.idle()
            assertEquals(1,f.prepares);assertEquals(1,f.metadataCalls);assertEquals(2,f.commits)
            assertEquals(id,f.controls.state.value.lastReceipt!!.transactionId);assertEquals(1,f.closedPlans)
        }finally{f.close()}
    }
    @Test fun acceptedReceiptIsKeptWhenPlanCloseOrRefreshFails()=runBlocking {
        val f=Fixture();try {
            f.open();f.failClose=true;f.failRefresh=true;assertTrue(f.controls.apply());f.idle()
            assertNotNull(f.controls.state.value.lastReceipt);assertNull(f.controls.state.value.retry)
            assertEquals(1,f.refreshes);assertEquals(1,f.closedPlans);assertTrue(f.controls.state.value.mayLeave)
        }finally{f.close()}
    }
    @Test fun cancelBeforeMutationGateCreatesNoPlanAndPreservesTyping()=runBlocking {
        val f=Fixture();try {
            f.open();f.controls.edit(f.frame().copy(outputWidth="20"));f.gate.lock()
            assertTrue(f.controls.apply());f.controls.cancel();f.gate.unlock();f.idle()
            assertEquals(0,f.prepares);assertTrue(f.controls.state.value.dirty);assertNull(f.controls.state.value.retry)
        }finally{if(f.gate.isLocked)f.gate.unlock();f.close()}
    }
    @Test fun lockedFrameAndToneCannotPrepareOrRemove()=runBlocking {
        val f=Fixture();try {
            f.frameEntry=ImageOutputEntry(id(3),id(4),true,true,ImageOutputSettings(0u,0u,64u,64u,0u,64u,64u))
            f.tones=listOf(ImageToneEntry(id(5),id(6),true,1.0,true,ImageToneSettings()))
            f.open();assertFalse(f.controls.apply());assertFalse(f.controls.remove())
            assertTrue(f.controls.selectTone(id(6)));assertFalse(f.controls.apply());assertFalse(f.controls.remove())
            assertEquals(0,f.prepares)
        }finally{f.close()}
    }
    @Test fun previewCoalescesLatestAndNeverPublishesEarlierOptions()=runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        try {
            f.open();f.pixelGate={if(f.pixels==1){entered.complete(Unit);release.await()}}
            assertTrue(f.controls.preview(0u,0u,true,false));entered.await()
            assertTrue(f.controls.preview(1u,2u,true,false));assertTrue(f.controls.preview(3u,4u,false,true))
            release.complete(Unit);f.pixelsIdle()
            assertEquals(2,f.pixels);assertEquals(3u,f.controls.state.value.preview!!.x)
            assertFalse(f.controls.state.value.previewMarked);assertTrue(f.controls.state.value.previewAssumeSrgb)
        }finally{release.complete(Unit);f.close()}
    }
    @Test fun oldPreviewCannotReturnAfterOpsAndDocSwitch()=runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        try {
            f.open();f.pixelGate={entered.complete(Unit);release.await()};f.controls.preview(0u,0u,true,true);entered.await()
            f.controls.observe(1uL);f.documentId=id(22);f.publish(1uL,epoch=1)
            release.complete(Unit);f.idle();assertNull(f.controls.state.value.preview)
            assertEquals(id(22),f.controls.state.value.snapshot!!.binding.documentId)
        }finally{release.complete(Unit);f.close()}
    }
    @Test fun foreignPreviewBindingOrLengthIsRefused()=runBlocking {
        val f=Fixture();try {
            f.open();f.corruptPixels=true;f.controls.preview(0u,0u,true,true);f.pixelsIdle()
            assertNull(f.controls.state.value.preview);assertNotNull(f.controls.state.value.issue)
        }finally{f.close()}
    }
    @Test fun closingWaitsForAnUncancellableReaderAndNeverDisplaysItsPixels()=runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();var close:Deferred<Unit>?=null
        try {
            f.open();f.pixelGate={withContext(NonCancellable){entered.complete(Unit);release.await()}}
            f.controls.preview(0u,0u,true,true);entered.await()
            close=async(start=CoroutineStart.UNDISPATCHED){f.controls.close()};assertFalse(close.isCompleted)
            release.complete(Unit);withTimeout(3000){close.await()};assertNull(f.controls.state.value.preview)
            val before=f.capabilities;f.controls.active(true);f.controls.requestExportContext();f.publish()
            assertFalse(f.controls.preview(0u,0u,true,true));assertFalse(f.controls.discardRetry());assertEquals(before,f.capabilities)
        }finally{release.complete(Unit);withContext(NonCancellable){close?.join();f.close()}}
    }
    @Test fun callerCancellationPreventsLateCapabilitiesAndReplacementWork()=runBlocking {
        val f=Fixture();try {
            f.open();f.scope.cancel();val before=f.capabilities
            f.controls.requestExportContext();assertFalse(f.controls.apply());assertFalse(f.controls.preview(0u,0u,true,true))
            assertEquals(before,f.capabilities)
        }finally{f.close()}
    }

    private class Fixture {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined);val gate=Mutex()
        var info=ProjectInfo(id(1),"Output",id(2),1uL,false,false,0uL,"a".repeat(64),listOf(id(20)))
        var documentId=id(20);var frameEntry:ImageOutputEntry?=null;var tones=emptyList<ImageToneEntry>()
        var capabilities=0;var reads=0;var prepares=0;var commits=0;var closedPlans=0;var ids=0;var metadataCalls=0;var refreshes=0;var pixels=0
        var readGate:(suspend()->Unit)?=null;var metadataGate:(suspend()->Unit)?=null;var commitGate:(suspend()->Unit)?=null;var pixelGate:(suspend()->Unit)?=null
        var failCommit=false;var failClose=false;var failRefresh=false;var corruptPixels=false
        val commands=mutableListOf<ImageOutputCommand>()
        val project=object:UnusedProject(){}
        val controls=ImageOutputInteraction(scope,gate,{ids++;id(100+ids)},
            {metadataCalls++;metadataGate?.invoke();WorkflowMetadata(id(60),info.deviceId,info.nextLamport,1700000000000)},
            {true},{refreshes++;if(failRefresh)throw IllegalStateException("refresh")},{
                capabilities++;object:WorkbenchImageOutputs {
                    override suspend fun snapshot(documentId:String,memoryBudgetBytes:ULong):ImageOutputSnapshot {
                        reads++;val result=this@Fixture.snapshot();readGate?.invoke();return result
                    }
                    override suspend fun prepare(binding:WorkflowBinding,metadata:WorkflowMetadata,command:ImageOutputCommand,memoryBudgetBytes:ULong):WorkbenchWorkflowPlan {
                        prepares++;commands+=command
                        return object:WorkbenchWorkflowPlan {
                            var closed=false
                            override fun describe()=WorkflowPlanInfo(metadata.transactionId,binding,1u,metadata.firstLamport+1uL)
                            override suspend fun commit():WorkflowReceipt {commits++;commitGate?.invoke();if(failCommit)throw WorkflowFailure(WorkflowFailureKind.Storage);return WorkflowReceipt(metadata.transactionId,info,commits>1)}
                            override fun close(){if(!closed){closed=true;closedPlans++;if(failClose)throw IllegalStateException("close")}}
                        }
                    }
                    override suspend fun preview(binding:WorkflowBinding,region:SelectionRegion,marked:Boolean,assumeUntaggedSrgb:Boolean,memoryBudgetBytes:ULong):ImageOutputPreview {
                        pixels++;val source=this@Fixture.snapshot();pixelGate?.invoke()
                        return ImageOutputPreview(binding,source.sourceAssetId,source.output?.takeIf{it.enabled}?.settings,
                            source.outputWidth,source.outputHeight,region.x,region.y,region.width,region.height,
                            ByteArray(if(corruptPixels)1 else (region.width*region.height*4u).toInt()))
                    }
                }
            })
        fun snapshot()=ImageOutputSnapshot(WorkflowBinding(info.projectId,documentId,info.hostSeq,info.stateHash),"f".repeat(64),64u,64u,8u,frameEntry,tones)
        fun document()=DocumentSnapshot(documentId,"Output",64u,64u,8u,emptyList(),RenderList(info,emptyList()))
        fun publish(sequence:ULong=0uL,epoch:Long=0){controls.bind(project,document(),epoch,sequence)}
        fun frame()=controls.state.value.form as ImageOutputForm.Frame
        suspend fun open(){publish();controls.active(true);idle()}
        suspend fun idle(){withTimeout(3000){while(controls.state.value.loading || controls.state.value.busy)yield()}}
        suspend fun pixelsIdle(){withTimeout(3000){while(controls.state.value.previewLoading)yield()}}
        suspend fun close(){controls.close();scope.cancel()}
    }
    private abstract class UnusedProject:WorkbenchProject {
        override val changes=emptyFlow<ProjectChange>()
        override suspend fun info():ProjectInfo=error("unused")
        override suspend fun document(documentId:String):DocumentSnapshot=error("unused")
        override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("unused")
        override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("unused")
        override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("unused")
        override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("unused")
        override suspend fun render(documentId:String,rectangle:Rect?):RenderList=error("unused")
        override suspend fun export(options:ExportOptions):ExportResult=error("unused")
        override suspend fun close()=Unit
    }
    companion object{private fun id(n:Int)="00000000-0000-4000-8000-"+n.toString().padStart(12,'0')}
}
