package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test

class DocumentControlsInteractionTest {
    @Test fun capabilityIsLazyAndTwentyDocumentsAreNotTruncated() = runBlocking {
        val f=Fixture()
        try {
            f.publish(); repeat(3){yield()}; assertEquals(0,f.capabilities)
            f.controls.active(true); f.idle()
            assertEquals(20,f.controls.state.value.snapshot!!.documents.size)
            assertEquals(1,f.reads)
        } finally { f.close() }
    }
    @Test fun completeImmutableCommandMakesExactlyOnePlanAndOneCommit() = runBlocking {
        val f=Fixture()
        try {
            f.open()
            val ids=mutableListOf(id(7),id(8))
            f.gate.lock()
            assertTrue(f.controls.execute(DocumentControlCommand.Group(id(9),ids)))
            ids.clear()
            assertFalse(f.controls.execute(DocumentControlCommand.RenameDocument("second")))
            f.gate.unlock(); f.idle()
            assertEquals(listOf(id(7),id(8)),(f.commands.single() as DocumentControlCommand.Group).memberIds)
            assertEquals(1,f.prepares); assertEquals(1,f.commits); assertEquals(1,f.closedPlans)
            assertNotNull(f.controls.state.value.lastReceipt)
        } finally { if(f.gate.isLocked)f.gate.unlock(); f.close() }
    }
    @Test fun delayedReadCannotReintroduceOldVisibleRevisionAfterOps() = runBlocking {
        val f=Fixture(); val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        try {
            f.readGate={entered.complete(Unit);release.await()}
            f.publish();f.controls.active(true);entered.await()
            f.controls.observe(1uL)
            release.complete(Unit);repeat(3){yield()}
            assertNull(f.controls.state.value.snapshot)
            f.readGate=null;f.info=f.info.copy(stateHash="b".repeat(64));f.publish(1uL);f.idle()
            assertEquals(f.info.stateHash,f.controls.state.value.snapshot!!.binding.stateHash)
        } finally { release.complete(Unit);f.close() }
    }
    @Test fun duplicateEventStillRebindsACompleteCurrentSnapshot() = runBlocking {
        val f=Fixture()
        try { f.open();f.controls.observe(1uL);f.publish(1uL);f.idle();assertTrue(f.controls.state.value.ready) }
        finally { f.close() }
    }
    @Test fun metadataSuspensionAndChangedVisibleHashRefuseBeforePreparing() = runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        try {
            f.open();f.metadataGate={entered.complete(Unit);release.await()}
            assertTrue(f.controls.execute(DocumentControlCommand.RenameDocument("first")));entered.await()
            f.info=f.info.copy(stateHash="c".repeat(64));f.publish(1uL)
            release.complete(Unit);f.idle();assertEquals(0,f.prepares);assertEquals(0,f.commits)
        } finally { release.complete(Unit);f.close() }
    }
    @Test fun cancelledProducerRemainsOwnedAcrossReopenUntilDetachSettles() = runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        var detach:Deferred<Unit>?=null
        try {
            f.open();f.commitGate={withContext(NonCancellable){entered.complete(Unit);release.await()};currentCoroutineContext().ensureActive()}
            assertTrue(f.controls.execute(DocumentControlCommand.RenameDocument("first")));entered.await()
            f.controls.cancel();f.controls.active(false);f.controls.active(true);f.controls.reload()
            assertTrue(f.controls.state.value.busy)
            assertFalse(f.controls.execute(DocumentControlCommand.RenameDocument("overlap")))
            detach=async(start=CoroutineStart.UNDISPATCHED){f.controls.detach()}
            assertFalse(detach.isCompleted);assertEquals(0,f.closedPlans)
            release.complete(Unit);withTimeout(3000){detach.await()}
            assertEquals(1,f.closedPlans);assertFalse(f.controls.state.value.busy);assertNull(f.controls.state.value.retry)
        } finally { release.complete(Unit);withContext(NonCancellable){detach?.join();f.close()} }
    }
    @Test fun uncertainCommitKeepsTheExactPlanAndRetryNeverGeneratesAnotherTransaction() = runBlocking {
        val f=Fixture()
        try {
            f.open();f.failFirstCommit=true
            assertTrue(f.controls.execute(DocumentControlCommand.RenameDocument("first")));f.idle()
            val retained=f.controls.state.value.retry!!
            assertFalse(f.controls.execute(DocumentControlCommand.RenameDocument("new")))
            assertFalse(f.controls.state.value.mayLeave)
            assertTrue(f.controls.retry());f.idle()
            assertEquals(1,f.prepares);assertEquals(1,f.metadataCalls);assertEquals(2,f.commits)
            assertEquals(retained.transactionId,f.controls.state.value.lastReceipt!!.transactionId)
            assertTrue(f.controls.state.value.lastReceipt!!.duplicate);assertEquals(1,f.closedPlans)
        } finally { f.close() }
    }
    @Test fun acceptedReceiptSurvivesFailedRefreshAndDoesNotBecomeAnUncertainRetry() = runBlocking {
        val f=Fixture()
        try {
            f.open();f.failRefresh=true
            assertTrue(f.controls.execute(DocumentControlCommand.RenameDocument("saved")));f.idle()
            assertNotNull(f.controls.state.value.lastReceipt);assertNull(f.controls.state.value.retry)
            assertEquals(1,f.closedPlans);assertTrue(f.controls.state.value.mayLeave)
        } finally { f.close() }
    }
    @Test fun metadataChoiceDoesNotRedirectPenAfterPeerLocksThatLayer() = runBlocking {
        val f=Fixture()
        try {
            f.open();f.controls.drawingLayer(id(4));assertEquals(id(4),f.controls.state.value.drawingLayerId)
            f.layers=f.layers.map{if(it.layerId==id(4))it.copy(locked=true)else it}
            f.info=f.info.copy(stateHash="d".repeat(64));f.publish(1uL);f.idle()
            assertEquals(id(4),f.controls.state.value.drawingLayerId)
            assertNull(documentControlDrawingLayer(f.document(),id(4)))
            assertEquals(id(3),documentControlDrawingLayer(f.document(),id(3))!!.id)
        } finally { f.close() }
    }
    @Test fun closedControllerCannotAcquireCapabilitiesOrMutateOnLateUiCallbacks() = runBlocking {
        val f=Fixture()
        try {
            f.open();f.controls.close();val before=f.capabilities
            f.publish();f.controls.active(true);f.controls.reload()
            assertFalse(f.controls.execute(DocumentControlCommand.RenameDocument("late")))
            repeat(3){yield()};assertEquals(before,f.capabilities)
        } finally { f.close() }
    }
    @Test fun cancelledParentCannotAcquireAReplacementOperationBeforeExplicitDisposal() = runBlocking {
        val f=Fixture()
        try {
            f.open();f.scope.cancel();val before=f.capabilities
            f.controls.active(false);f.controls.active(true);f.controls.reload()
            assertFalse(f.controls.execute(DocumentControlCommand.RenameDocument("late")))
            repeat(3){yield()};assertEquals(before,f.capabilities);assertEquals(0,f.prepares)
        } finally { f.close() }
    }
    @Test fun lateAcceptedReceiptFromAnOldAttachmentCannotRefreshItsReplacement() = runBlocking {
        val f=Fixture();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        try {
            f.open();f.commitGate={withContext(NonCancellable){entered.complete(Unit);release.await()}}
            assertTrue(f.controls.execute(DocumentControlCommand.RenameDocument("old")));entered.await()
            // Defensive contract: normal app switching first awaits detach,
            // but even a caller that rebinds early cannot publish the old edit.
            f.controls.bind(f.project,f.document(),1,0uL)
            release.complete(Unit);f.idle()
            assertNull(f.controls.state.value.lastReceipt);assertEquals(0,f.refreshCalls)
            assertEquals(1,f.commits);assertEquals(1,f.closedPlans)
        } finally { release.complete(Unit);f.close() }
    }

    private class Fixture {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val gate=Mutex()
        var info=ProjectInfo(id(1),"Project",id(5),1uL,false,false,0uL,"a".repeat(64),(20..39).map(::id))
        var layers=listOf(LayerControlRow(id(3),"Ink","annotation","V",true,false,1.0,"normal",2u),
            LayerControlRow(id(4),"Other","annotation","W",true,false,1.0,"normal",0u))
        var capabilities=0;var reads=0;var prepares=0;var commits=0;var closedPlans=0;var metadataCalls=0;var refreshCalls=0
        var readGate:(suspend()->Unit)?=null;var commitGate:(suspend()->Unit)?=null;var metadataGate:(suspend()->Unit)?=null
        var failFirstCommit=false;var failRefresh=false
        val commands=mutableListOf<DocumentControlCommand>()
        val project=object:UnsupportedProject(){override suspend fun info()=this@Fixture.info}
        val controls=DocumentControlsInteraction(scope,gate,{
            metadataCalls++;metadataGate?.invoke();WorkflowMetadata(id(60),info.deviceId,info.nextLamport,1700000000000)
        },{true},{refreshCalls++;if(failRefresh)throw IllegalStateException("refresh")},{
            capabilities++
            object:WorkbenchDocumentControls {
                override suspend fun snapshot(documentId:String,memoryBudgetBytes:ULong):DocumentControlSnapshot {
                    reads++;val result=this@Fixture.snapshot();readGate?.invoke();return result
                }
                override suspend fun prepare(binding:WorkflowBinding,metadata:WorkflowMetadata,command:DocumentControlCommand,memoryBudgetBytes:ULong):WorkbenchWorkflowPlan {
                    prepares++;commands+=command
                    return object:WorkbenchWorkflowPlan {
                        var closed=false
                        override fun describe()=WorkflowPlanInfo(metadata.transactionId,binding,1u,metadata.firstLamport+1uL)
                        override suspend fun commit():WorkflowReceipt {
                            commits++;commitGate?.invoke()
                            if(failFirstCommit&&commits==1)throw WorkflowFailure(WorkflowFailureKind.Storage)
                            return WorkflowReceipt(metadata.transactionId,info,duplicate=commits>1)
                        }
                        override fun close(){if(!closed){closed=true;closedPlans++}}
                    }
                }
            }
        })
        fun document()=DocumentSnapshot(id(20),"Current",64u,64u,8u,layers.map{LayerInfo(it.layerId,it.name,it.visible,it.locked,it.opacity,it.blend)},RenderList(info,emptyList()))
        fun snapshot()=DocumentControlSnapshot(WorkflowBinding(info.projectId,id(20),info.hostSeq,info.stateHash),info,
            (20..39).map{DocumentControlRow(id(it),"Document $it","image",64u,64u)},layers,
            listOf(7,8).map{ObjectControlRow(id(it),id(3),null,"V","line",false,false,Transform())},emptyList())
        fun publish(sequence:ULong=0uL){controls.bind(project,document(),0,sequence)}
        suspend fun open(){publish();controls.active(true);idle()}
        suspend fun idle(){withTimeout(3000){while(controls.state.value.busy||controls.state.value.loading)yield()}}
        suspend fun close(){controls.close();scope.cancel()}
    }
    private abstract class UnsupportedProject:WorkbenchProject {
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
    companion object { private fun id(n:Int)="00000000-0000-4000-8000-"+n.toString().padStart(12,'0') }
}
