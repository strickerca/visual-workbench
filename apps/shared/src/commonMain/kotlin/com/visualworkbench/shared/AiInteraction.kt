package com.visualworkbench.shared
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
public enum class AiStage { Idle, Preparing, Sending, Comparing, Accepting, Saving, Closed }
public data class AiInteractionState(public val stage:AiStage=AiStage.Idle,public val review:AiReview?=null,
    public val candidate:AiCandidateInfo?=null,public val failure:AiFailureKind?=null,public val receipt:AiResultReceipt?=null)
/** One project-scoped controller. The application separately owns service. The
 * caller must close this owner before closing/detaching the project or service. */
public class AiInteraction private constructor(private val edits:()->WorkbenchAiEdits,scope:CoroutineScope){
    public constructor(project:WorkbenchProject,service:WorkbenchAiService,scope:CoroutineScope):this({project.aiEdits(service)},scope)
    internal constructor(edits:WorkbenchAiEdits,scope:CoroutineScope):this({edits},scope)
    private val lifetime=SupervisorJob(scope.coroutineContext[Job])
    private val ownerScope=CoroutineScope(scope.coroutineContext+lifetime)
    private val guard=Mutex()
    private var closed=false
    private var operation:Deferred<*>?=null
    private var request:WorkbenchAiRequest?=null
    private val mutable=MutableStateFlow(AiInteractionState())
    public val state:StateFlow<AiInteractionState> = mutable.asStateFlow()
    private suspend fun <T> run(stage:AiStage,body:suspend()->T):T{
        val job=guard.withLock{
            if(closed)throw AiFailure(AiFailureKind.Closed)
            if(operation!=null)throw AiFailure(AiFailureKind.Busy)
            ownerScope.async(start=CoroutineStart.LAZY){
                mutable.value=mutable.value.copy(stage=stage,failure=null)
                try{body()}
                catch(error:AiFailure){mutable.value=mutable.value.copy(failure=error.kind);throw error}
                finally{withContext(NonCancellable){// The awaiter clears only this operation, never a newer task.
                    guard.withLock{if(!closed)mutable.value=mutable.value.copy(stage=AiStage.Idle)}
                }}
            }.also{operation=it}
        }
        job.start()
        try{return job.await()}
        catch(error:CancellationException){job.cancel();withContext(NonCancellable){job.join()};throw error}
        finally{withContext(NonCancellable){guard.withLock{if(operation===job)operation=null}}}
    }
    public suspend fun prepare(options:AiPrepareOptions):AiReview=run(AiStage.Preparing){
        withContext(NonCancellable){request?.close();request=null}
        mutable.value=AiInteractionState(stage=AiStage.Preparing)
        var acquired:WorkbenchAiRequest?=null
        try{val next=edits().prepare(options);acquired=next
            currentCoroutineContext().ensureActive()
            guard.withLock{if(closed)throw AiFailure(AiFailureKind.Closed);request=next;acquired=null}
            mutable.value=mutable.value.copy(review=next.review);next.review
        }finally{withContext(NonCancellable){acquired?.close()}}
    }
    public suspend fun send(requestId:String,displayedEstimateMicrousd:ULong,acknowledgeSoftBudget:Boolean):AiCandidateInfo=run(AiStage.Sending){
        val active=request?:throw AiFailure(AiFailureKind.Invalid);val review=active.review
        if(review.requestId!=requestId || review.estimatedMicrousd!=displayedEstimateMicrousd)throw AiFailure(AiFailureKind.Stale)
        active.send(requestId,displayedEstimateMicrousd,acknowledgeSoftBudget).also{mutable.value=mutable.value.copy(candidate=it,receipt=null)}
    }
    public suspend fun pixels(mode:AiCompareMode,region:AiRegion):AiPixels=run(AiStage.Comparing){
        val id=mutable.value.candidate?.candidateId?:throw AiFailure(AiFailureKind.Invalid)
        (request?:throw AiFailure(AiFailureKind.Invalid)).compare(id,mode,region)
    }
    public suspend fun acceptBrush(brush:AiAcceptanceBrush):AiCandidateInfo=run(AiStage.Accepting){
        (request?:throw AiFailure(AiFailureKind.Invalid)).acceptBrush(brush).also{mutable.value=mutable.value.copy(candidate=it,receipt=null)}
    }
    /** Reread the retained paid result after cancelled DTO delivery. This does
     * not send, retry or authorize a provider request. */
    public suspend fun recoverCandidate():AiCandidateInfo=run(AiStage.Comparing){
        (request?:throw AiFailure(AiFailureKind.Invalid)).candidate().also{
            mutable.value=mutable.value.copy(candidate=it,receipt=null)
        }
    }
    public suspend fun save(options:AiSaveOptions):AiResultReceipt=run(AiStage.Saving){
        val id=mutable.value.candidate?.candidateId?:throw AiFailure(AiFailureKind.Invalid)
        (request?:throw AiFailure(AiFailureKind.Invalid)).save(id,options).also{mutable.value=mutable.value.copy(receipt=it)}
    }
    public suspend fun cancel(){val job=guard.withLock{operation};job?.cancel();withContext(NonCancellable){job?.join();guard.withLock{if(operation===job)operation=null}}}
    public suspend fun close(){withContext(NonCancellable){
        val job=guard.withLock{closed=true;operation};job?.cancel();job?.join()
        guard.withLock{request?.close();request=null;operation=null;lifetime.cancel();mutable.value=mutable.value.copy(stage=AiStage.Closed)}
    }}
}
