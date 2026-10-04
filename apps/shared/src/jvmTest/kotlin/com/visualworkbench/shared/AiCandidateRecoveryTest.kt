package com.visualworkbench.shared

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class AiCandidateRecoveryTest {
    private val binding=WorkflowBinding("project","document",1uL,"a".repeat(64))
    private val review=AiReview("request",binding,"b".repeat(64),"c".repeat(64),"d".repeat(64),"e".repeat(64),"fixture","low",16u,16u,8u,16u,16u,0,0,16u,16u,0u,50uL,"fixture","2026-10-03","2026-11-02",false)
    private fun candidate(id:String)=AiCandidateInfo("request",id,binding,16u,16u,8u,id!="paid",AiSettlement.UsagePriced,50uL,AiProofInfo(0uL,"f".repeat(64),"f".repeat(64),null,null,null,0.0,0.0,1.0))
    private fun options()=AiPrepareOptions(binding,emptyList(),"intent","instruction","e".repeat(64),null)
    private inner class Request:WorkbenchAiRequest {
        override val review=this@AiCandidateRecoveryTest.review
        var retained=candidate("paid");var sends=0;var reads=0;var closed=0
        val accepted=CompletableDeferred<Unit>()
        var reading:CompletableDeferred<Unit>?=null
        var settleRead:CompletableDeferred<Unit>?=null
        override suspend fun send(requestId:String,displayedEstimateMicrousd:ULong,acknowledgeSoftBudget:Boolean):AiCandidateInfo { sends++;return retained }
        override suspend fun candidate():AiCandidateInfo {
            reads++;reading?.complete(Unit)
            // Model native cancellation settlement: the retained request cannot
            // be closed while its already-running producer is still active.
            settleRead?.let { withContext(NonCancellable){it.await()} }
            currentCoroutineContext().ensureActive();return retained
        }
        override suspend fun acceptBrush(brush:AiAcceptanceBrush):AiCandidateInfo {
            check(brush.expectedCandidateId==retained.candidateId)
            retained=candidate(brush.nextCandidateId);accepted.complete(Unit)
            awaitCancellation() // Native mutation completed; DTO delivery lost.
        }
        override suspend fun compare(candidateId:String,mode:AiCompareMode,region:AiRegion):AiPixels {
            check(candidateId==retained.candidateId)
            return AiPixels(candidateId,region,16u,16u,ByteArray(4))
        }
        override suspend fun save(candidateId:String,options:AiSaveOptions):AiResultReceipt=error("unused")
        override suspend fun close(){closed++}
    }
    private fun edits(request:Request)=object:WorkbenchAiEdits {
        override suspend fun prepare(options:AiPrepareOptions):WorkbenchAiRequest=request
        override suspend fun context(binding:WorkflowBinding,memoryBudgetBytes:ULong):AiContext=error("unused")
        override suspend fun resultPixels(binding:WorkflowBinding,resultId:String,region:AiRegion,memoryBudgetBytes:ULong):AiResultPixels=error("unused")
        override suspend fun resultStatus(binding:WorkflowBinding,resultId:String,accepted:Boolean,metadata:WorkflowMetadata):WorkflowReceipt=error("unused")
    }
    @Test fun cancelledPartialAcceptanceCanRecoverWithoutAnotherSend()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val request=Request();val owner=AiInteraction(edits(request),scope)
        try {
            owner.prepare(options());owner.send("request",50uL,false)
            val acceptance=launch { owner.acceptBrush(AiAcceptanceBrush("paid","partial",listOf(Point(1.0,1.0)),1.0)) }
            withTimeout(5000){request.accepted.await()};acceptance.cancelAndJoin()
            assertEquals("paid",owner.state.value.candidate?.candidateId)
            assertEquals("partial",owner.recoverCandidate().candidateId)
            assertEquals("partial",owner.state.value.candidate?.candidateId)
            assertEquals("partial",owner.pixels(AiCompareMode.After,AiRegion(0u,0u,1u,1u)).candidateId)
            assertEquals(1,request.sends);assertEquals(1,request.reads)
        } finally { owner.close();scope.cancel() }
    }
    @Test fun recoveryUsesOperationGateAndCloseWaitsForRetainedRead()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val request=Request();val owner=AiInteraction(edits(request),scope)
        val reading=CompletableDeferred<Unit>();val settle=CompletableDeferred<Unit>();request.reading=reading;request.settleRead=settle
        var recovering:Job?=null;var closing:Deferred<Unit>?=null
        try {
            owner.prepare(options())
            val recoveringNow=launch { owner.recoverCandidate() };recovering=recoveringNow
            withTimeout(5000){reading.await()}
            try { owner.recoverCandidate();fail("Concurrent recovery admitted") } catch(error:AiFailure){assertEquals(AiFailureKind.Busy,error.kind)}
            val closingNow=async(start=CoroutineStart.UNDISPATCHED){owner.close()};closing=closingNow
            assertFalse(closingNow.isCompleted);assertEquals(0,request.closed)
            settle.complete(Unit)
            withTimeout(5000){closingNow.await();recoveringNow.join()}
            assertEquals(1,request.closed);assertEquals(0,request.sends);assertEquals(AiStage.Closed,owner.state.value.stage)
        } finally {
            settle.complete(Unit)
            withContext(NonCancellable){recovering?.cancelAndJoin();closing?.await();owner.close()}
            scope.cancel()
        }
    }
}
