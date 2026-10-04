package com.visualworkbench.shared
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
class AiInteractionTest {
    private val binding=WorkflowBinding("project","document",1uL,"a".repeat(64))
    private fun review(id:String)=AiReview(id,binding,"b".repeat(64),"c".repeat(64),"d".repeat(64),"e".repeat(64),"fixture","low",16u,16u,8u,16u,16u,0,0,16u,16u,0u,50uL,"fixture","2026-10-03","2026-11-02",false)
    private fun options()=AiPrepareOptions(binding,emptyList(),"intent","instruction","e".repeat(64),null)
    private inner class Request(val id:String):WorkbenchAiRequest{
        override val review=review(id);var sends=0;var closed=0;var failure:AiFailureKind?=null
        var entered:CompletableDeferred<Unit>?=null;var finish:CompletableDeferred<Unit>?=null
        override suspend fun send(requestId:String,displayedEstimateMicrousd:ULong,acknowledgeSoftBudget:Boolean):AiCandidateInfo{sends++;entered?.complete(Unit);finish?.await();throw AiFailure(failure?:AiFailureKind.Provider)}
        override suspend fun candidate():AiCandidateInfo=error("unused")
        override suspend fun compare(candidateId:String,mode:AiCompareMode,region:AiRegion):AiPixels=error("unused")
        override suspend fun acceptBrush(brush:AiAcceptanceBrush):AiCandidateInfo=error("unused")
        override suspend fun save(candidateId:String,options:AiSaveOptions):AiResultReceipt=error("unused")
        override suspend fun close(){closed++}
    }
    private fun edits(request:WorkbenchAiRequest)=object:WorkbenchAiEdits{
        override suspend fun context(binding:WorkflowBinding,memoryBudgetBytes:ULong):AiContext=error("unused")
        override suspend fun prepare(options:AiPrepareOptions)=request
        override suspend fun resultPixels(binding:WorkflowBinding,resultId:String,region:AiRegion,memoryBudgetBytes:ULong):AiResultPixels=error("unused")
        override suspend fun resultStatus(binding:WorkflowBinding,resultId:String,accepted:Boolean,metadata:WorkflowMetadata):WorkflowReceipt=error("unused")
    }
    @Test fun typedFailureDoesNotCancelNonSupervisorApplicationScope(): Unit = runBlocking{
        val parent=Job();val scope=CoroutineScope(Dispatchers.Default+parent);val request=Request("review");val owner=AiInteraction(edits(request),scope)
        try{owner.prepare(options());assertAiFailure<AiFailure>{owner.send("review",50uL,false)};assertTrue(parent.isActive);assertEquals(AiFailureKind.Provider,owner.state.value.failure);assertEquals(1,request.sends)}finally{owner.close();parent.cancel()}
    }
    @Test fun staleDisplayedReviewCannotSendTheCurrentRequest(): Unit = runBlocking{
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val request=Request("current");val owner=AiInteraction(edits(request),scope)
        try{owner.prepare(options());assertAiFailure<AiFailure>{owner.send("old",50uL,false)};assertAiFailure<AiFailure>{owner.send("current",51uL,false)};assertEquals(0,request.sends)}finally{owner.close();scope.cancel()}
    }
    @Test fun closeCancelsOperationBeforeClosingItsRequest(): Unit = runBlocking{
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val request=Request("review");request.entered=CompletableDeferred();request.finish=CompletableDeferred();val owner=AiInteraction(edits(request),scope)
        try{owner.prepare(options());val sending=launch{try{owner.send("review",50uL,false)}catch(_:CancellationException){}};withTimeout(5000){request.entered!!.await()};withTimeout(5000){owner.close()};sending.join();assertEquals(1,request.closed);assertEquals(AiStage.Closed,owner.state.value.stage);assertAiFailure<AiFailure>{owner.prepare(options())}}finally{request.finish!!.complete(Unit);owner.close();scope.cancel()}
    }
}

private inline fun <reified T:Throwable> assertAiFailure(block:()->Unit):T {
    try { block() } catch(error:Throwable) { if(error is T)return error;throw AssertionError("Unexpected exception type",error) }
    throw AssertionError("Expected exception was not thrown")
}
