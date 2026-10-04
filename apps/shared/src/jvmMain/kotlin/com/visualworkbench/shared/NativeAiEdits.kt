package com.visualworkbench.shared

import com.visualworkbench.bindings.core.*
import com.visualworkbench.bindings.core.AiService as NService
import com.visualworkbench.bindings.core.AiRequest as NRequest
import com.visualworkbench.bindings.core.AiConfiguration as NConfiguration
import com.visualworkbench.bindings.core.AiTokenEstimate as NEstimate
import com.visualworkbench.bindings.core.AiPrepareOptions as NPrepare
import com.visualworkbench.bindings.core.AiReview as NReview
import com.visualworkbench.bindings.core.AiCandidateInfo as NCandidate
import com.visualworkbench.bindings.core.AiProofInfo as NProof
import com.visualworkbench.bindings.core.AiCompareMode as NMode
import com.visualworkbench.bindings.core.AiRegion as NRegion
import com.visualworkbench.bindings.core.AiAcceptanceBrush as NBrush
import com.visualworkbench.bindings.core.AiSaveOptions as NSave
import com.visualworkbench.bindings.core.SelectionVersion as NSelection
import com.visualworkbench.bindings.core.Point as NPoint
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import java.util.concurrent.atomic.AtomicBoolean

private val aiScope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
private val aiSlots=Semaphore(4)
private fun failure(error:AiEditException)=AiFailure(when(error){
    is AiEditException.Invalid->AiFailureKind.Invalid;is AiEditException.Stale->AiFailureKind.Stale
    is AiEditException.Limit->AiFailureKind.Limit;is AiEditException.Depth->AiFailureKind.Depth
    is AiEditException.Color->AiFailureKind.Color;is AiEditException.Estimate->AiFailureKind.Estimate
    is AiEditException.SoftBudget->AiFailureKind.SoftBudget;is AiEditException.Unresolved->AiFailureKind.Unresolved
    is AiEditException.AttemptConsumed->AiFailureKind.AttemptConsumed;is AiEditException.Credentials->AiFailureKind.Credentials
    is AiEditException.Provider->AiFailureKind.Provider;is AiEditException.Proof->AiFailureKind.Proof
    is AiEditException.Storage->AiFailureKind.Storage;is AiEditException.Unsupported->AiFailureKind.Unsupported
    is AiEditException.Cancelled->AiFailureKind.Cancelled;is AiEditException.Closed->AiFailureKind.Closed
    is AiEditException.Busy->AiFailureKind.Busy})
private inline fun <T> direct(block:()->T):T=try{block()}catch(e:AiEditException){throw failure(e)}catch(e:IllegalStateException){throw AiFailure(AiFailureKind.Closed)}
/** A bounded independently-owned producer, including async disposal of late
 * handles. Cancellation never frees the token/slot before native settlement. */
internal suspend fun <T> settledAi(release:suspend(T)->Unit={},block:suspend(Cancellation)->T):T =
    ownAiResult(aiScope,aiSlots,::Cancellation,{it.cancel()},{it.destroy()},release){token->try{block(token)}catch(e:AiEditException){throw failure(e)}catch(e:IllegalStateException){throw AiFailure(AiFailureKind.Closed)}}
private fun NConfiguration.public()=AiConfiguration(json,fingerprint,verifiedOn,expiresOn,model,quality,dailySoftBudgetMicrousd)
private fun AiTokenEstimate.native()=NEstimate(schema,textInput,imageInput,imageOutput,provenance,verifiedOn,expiresOn)
private fun AiPrepareOptions.native()=NPrepare(workflowBinding(binding),selections.map{NSelection(it.objectId,it.assetId,it.version)},intentId,instruction,configurationFingerprint,estimate?.native(),featherPx,allow16bitProviderCopy,assumeUntaggedSrgb,memoryBudgetBytes)
private fun NReview.public()=AiReview(requestId,workflowBinding(binding),sourceAssetId,sourceFileSha256,maskSha256,configurationFingerprint,model,quality,sourceWidth,sourceHeight,sourceBitDepth,modelWidth,modelHeight,cropX,cropY,cropWidth,cropHeight,featherPx,estimatedMicrousd,estimateProvenance,estimateExpiresOn,configurationExpiresOn,providerCopyReducesDepth)
private fun NProof.public()=AiProofInfo(changedOutside,outsideSha256Before,outsideSha256After,changedUnaccepted,unacceptedSha256Before,unacceptedSha256After,deltaE2000Mean,deltaE2000Max,ssimInsideMask)
private fun NCandidate.public()=AiCandidateInfo(requestId,candidateId,workflowBinding(binding),width,height,bitDepth,partial,when(settlement){com.visualworkbench.bindings.core.AiSettlement.USAGE_PRICED->AiSettlement.UsagePriced;com.visualworkbench.bindings.core.AiSettlement.USAGE_MISSING->AiSettlement.UsageMissing;com.visualworkbench.bindings.core.AiSettlement.LEDGER_UNAVAILABLE->AiSettlement.LedgerUnavailable},actualMicrousd,proof.public())
private fun AiRegion.native()=NRegion(x,y,width,height)
private fun NRegion.public()=AiRegion(x,y,width,height)
private fun AiCompareMode.native():NMode=when(this){AiCompareMode.Before->NMode.Before;AiCompareMode.After->NMode.After;is AiCompareMode.Wipe->NMode.Wipe(vertical,cut,resultBefore);AiCompareMode.Split->NMode.Split;is AiCompareMode.Difference->NMode.Difference(gain)}
public actual fun defaultAiConfiguration():AiConfiguration{prepareCoreLibrary();return direct{com.visualworkbench.bindings.core.defaultAiConfiguration().public()}}
public actual fun validateAiConfiguration(json:String):AiConfiguration{if(json.length>65_536)throw AiFailure(AiFailureKind.Limit);prepareCoreLibrary();return direct{com.visualworkbench.bindings.core.validateAiConfiguration(json).public()}}
public actual suspend fun createAiService(applicationPrivateDirectory:String,provisionFirstInstall:Boolean):WorkbenchAiService{
    prepareCoreLibrary()
    return settledAi(release={it.close()}){cancel->NativeAiService(com.visualworkbench.bindings.core.createAiService(applicationPrivateDirectory,provisionFirstInstall,cancel))}
}
private class NativeAiService(val handle:NService):WorkbenchAiService{
    private val closed=AtomicBoolean(false)
    fun check(){if(closed.get())throw AiFailure(AiFailureKind.Closed)}
    override suspend fun configuration():AiConfiguration{check();return settledAi{handle.configuration(it).public()}}
    override suspend fun configure(json:String,expectedFingerprint:String):AiConfiguration{check();if(json.length>65_536||expectedFingerprint.length!=64)throw AiFailure(AiFailureKind.Invalid);return settledAi{handle.configure(json,expectedFingerprint,it).public()}}
    override suspend fun configureDailyBudget(expectedFingerprint:String,dailySoftBudgetMicrousd:ULong):AiConfiguration{check();return settledAi{handle.configureDailyBudget(expectedFingerprint,dailySoftBudgetMicrousd,it).public()}}
    override suspend fun readiness():AiReadiness{check();return settledAi{handle.readiness(it).let{v->AiReadiness(v.configured,v.trustReady,v.configurationCurrent,v.spentTodayMicrousd,v.dailySoftBudgetMicrousd)}}}
    override suspend fun saveWindowsKey(bytes:ByteArray){try{check();if(bytes.isEmpty()||bytes.size>4096)throw AiFailure(AiFailureKind.Invalid);settledAi{handle.saveWindowsKey(bytes,it)}}finally{bytes.fill(0)}}
    override suspend fun removeWindowsKey(){check();settledAi{handle.removeWindowsKey(it)}}
    override suspend fun close(){releaseNative{if(!closed.getAndSet(true))try{handle.shutdown()}finally{handle.destroy()}}}
}
public actual fun WorkbenchProject.aiEdits(service:WorkbenchAiService):WorkbenchAiEdits{
    val owner=service as? NativeAiService?:throw AiFailure(AiFailureKind.Invalid);owner.check()
    return NativeAiEdits(this,owner)
}
public actual fun WorkbenchProject.aiResults():WorkbenchAiResults=NativeAiResults(this)
private open class NativeAiResults(protected val project:WorkbenchProject):WorkbenchAiResults{
    override suspend fun context(binding:WorkflowBinding,memoryBudgetBytes:ULong):AiContext=settledAi{cancel->nativeProjectHandle(project).aiContext(workflowBinding(binding),memoryBudgetBytes,cancel).let{AiContext(workflowBinding(it.binding),it.sourceAssetId,it.width,it.height,it.bitDepth,it.changeSelections.map{s->SelectionVersion(s.objectId,s.assetId,s.version)},it.instructions.map{s->AiInstructionSummary(s.id,s.role,s.text,s.targetObjectIds,s.markerNumbers)})}}
    override suspend fun resultPixels(binding:WorkflowBinding,resultId:String,region:AiRegion,memoryBudgetBytes:ULong):AiResultPixels=settledAi{cancel->nativeProjectHandle(project).aiResultPixels(workflowBinding(binding),resultId,region.native(),memoryBudgetBytes,cancel).let{AiResultPixels(workflowBinding(it.binding),it.resultId,it.sourceAssetId,it.compositeAssetId,it.region.public(),it.canvasWidth,it.canvasHeight,it.rgbaSrgb)}}
    override suspend fun resultStatus(binding:WorkflowBinding,resultId:String,accepted:Boolean,metadata:WorkflowMetadata):WorkflowReceipt=settledAi{cancel->nativeProjectHandle(project).aiResultStatus(workflowBinding(binding),resultId,accepted,workflowMetadata(metadata),cancel).let{WorkflowReceipt(it.transactionId,workflowInfo(it.revision),it.duplicate)}}
}
private class NativeAiEdits(project:WorkbenchProject,val service:NativeAiService):NativeAiResults(project),WorkbenchAiEdits{
    override suspend fun prepare(options:AiPrepareOptions):WorkbenchAiRequest{
        service.check()
        if(options.selections.size !in 1..64 || options.instruction.length>16*1024 || options.estimate?.provenance?.length?.let{it>1024}==true)throw AiFailure(AiFailureKind.Invalid)
        return settledAi(release={it.close()}){cancel->
            val handle=nativeProjectHandle(project).prepareAiEdit(service.handle,options.native(),cancel)
            try{NativeAiRequest(handle)}catch(e:Throwable){try{handle.shutdown()}finally{handle.destroy()};throw e}
        }
    }
}
private class NativeAiRequest(val handle:NRequest):WorkbenchAiRequest{
    private val closed=AtomicBoolean(false)
    override val review:AiReview=direct{handle.describe().public()}
    private fun check(){if(closed.get())throw AiFailure(AiFailureKind.Closed)}
    override suspend fun send(requestId:String,displayedEstimateMicrousd:ULong,acknowledgeSoftBudget:Boolean):AiCandidateInfo{check();return settledAi{handle.send(requestId,displayedEstimateMicrousd,acknowledgeSoftBudget,it).public()}}
    override suspend fun candidate():AiCandidateInfo{check();return settledAi{handle.candidate(it).public()}}
    override suspend fun compare(candidateId:String,mode:AiCompareMode,region:AiRegion):AiPixels{check();return settledAi{handle.compare(candidateId,mode.native(),region.native(),it).let{p->AiPixels(p.candidateId,p.region.public(),p.canvasWidth,p.canvasHeight,p.rgbaSrgb)}}}
    override suspend fun acceptBrush(brush:AiAcceptanceBrush):AiCandidateInfo{check();if(brush.points.size !in 1..16_384)throw AiFailure(AiFailureKind.Limit);return settledAi{handle.acceptBrush(NBrush(brush.expectedCandidateId,brush.nextCandidateId,brush.points.map{p->NPoint(p.x,p.y)},brush.radius,brush.opacity,brush.subtract,brush.clearFirst),it).public()}}
    override suspend fun save(candidateId:String,options:AiSaveOptions):AiResultReceipt{check();return settledAi{cancel->handle.save(candidateId,NSave(workflowBinding(options.expected),workflowMetadata(options.metadata),options.resultId,options.layerId,options.objectId,options.replaceObjectId),cancel).let{AiResultReceipt(it.transactionId,it.resultId,it.objectId,it.layerId,it.outputAssetId,it.compositeAssetId,it.acceptanceMaskAssetId,workflowInfo(it.revision),it.proof.public())}}}
    override suspend fun close(){releaseNative{if(!closed.getAndSet(true))try{handle.shutdown()}finally{handle.destroy()}}}
}
