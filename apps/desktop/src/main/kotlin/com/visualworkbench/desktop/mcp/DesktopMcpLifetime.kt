package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.WorkbenchCore
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import java.nio.file.Path
import java.util.concurrent.atomic.AtomicReference

internal interface McpOwnedService { suspend fun close() }
/** The start producer owns its result until publication under the same lock as
 * close admission. Every close joins that producer and the actual owner cleanup. */
internal class McpStartLifetime<T:McpOwnedService>(private val create:suspend()->T) {
    private val lock=Any()
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    private var starting:Deferred<T>?=null
    private var value:T?=null
    private var closed=false
    private var lateCleanupFailure:Throwable?=null
    private val stopped=CompletableDeferred<Unit>()
    suspend fun start():T {
        val task=synchronized(lock){
            check(!closed);value?.let{return it}
            starting?:scope.async(start=CoroutineStart.LAZY){
                var acquired:T?=null
                try{
                    val result=create();acquired=result
                    synchronized(lock){if(closed)throw CancellationException("MCP owner closed");value=result;acquired=null}
                    result
                }catch(error:Throwable){synchronized(lock){if(closed&&error !is CancellationException)lateCleanupFailure=error};throw error}
                finally{withContext(NonCancellable){try{acquired?.close()}catch(error:Throwable){synchronized(lock){lateCleanupFailure=error};throw error}}}
            }.also{starting=it;it.start()}
        }
        try{return task.await()}
        finally{if(task.isCompleted)synchronized(lock){if(starting===task)starting=null}}
    }
    suspend fun close(){withContext(NonCancellable){
        val first=synchronized(lock){if(closed)false else{closed=true;true}}
        if(first){
            var error:Throwable?=null
            try{
                val producer=synchronized(lock){starting};producer?.cancel();producer?.join()
                val owned=synchronized(lock){starting=null;value.also{value=null}}
                try{owned?.close()}finally{synchronized(lock){lateCleanupFailure}?.let{throw it}}
            }catch(e:Throwable){error=e}
            finally{scope.cancel();if(error==null)stopped.complete(Unit)else stopped.completeExceptionally(error)}
        }
        stopped.await()
    }}
}
/** A stopped cycle is replaced only after its actual workers/resources settle.
 * Shutdown callers join the same completion; uncertain cleanup seals restarts. */
internal class McpServiceCycles<T:McpOwnedService>(private val create:suspend()->T) {
    private val lock=Any()
    private var lifetime:McpStartLifetime<T>?=null
    private var stopping:CompletableDeferred<Unit>?=null
    private var closed=false
    private var failure:Throwable?=null
    suspend fun start():T {
        val selected=synchronized(lock){check(!closed&&stopping==null);lifetime?:McpStartLifetime(create).also{lifetime=it}}
        val result=selected.start()
        synchronized(lock){if(closed||stopping!=null||lifetime!==selected)throw CancellationException("MCP cycle retired")}
        return result
    }
    suspend fun stop(){withContext(NonCancellable){
        var selected:McpStartLifetime<T>?=null
        var first=false
        val completion=synchronized(lock){
            failure?.let{throw it}
            stopping?:CompletableDeferred<Unit>().also{stopping=it;selected=lifetime;lifetime=null;first=true}
        }
        if(first){
            var error:Throwable?=null
            try{selected?.close()}catch(value:Throwable){error=value}
            synchronized(lock){if(error!=null){failure=error;closed=true};stopping=null}
            if(error==null)completion.complete(Unit)else completion.completeExceptionally(error)
        }
        completion.await()
    }}
    suspend fun close(){withContext(NonCancellable){synchronized(lock){closed=true};stop()}}
}
internal interface McpDesktopServiceOwner:McpOwnedService { val coordinator:DesktopMcpCoordinator }
internal class McpDesktopOwned(override val coordinator:DesktopMcpCoordinator,private val bundle:DesktopMcpBundle):McpDesktopServiceOwner {
    override suspend fun close(){withContext(NonCancellable){try{coordinator.close()}finally{bundle.close()}}}
}
internal data class McpLifetimeState(val starting:Boolean=false,val owner:DesktopMcpCoordinator?=null,val issue:String?=null,val closed:Boolean=false,
    val failureStage:McpFailureStage?=null)
internal class DesktopMcpLifetime internal constructor(create:suspend()->McpDesktopServiceOwner) {
    constructor(privateParent:Path,helper:Path,helperHash:String,core:WorkbenchCore,device:String):this({openMcpDesktopService(privateParent,helper,helperHash,core,device)})
    private val lock=Any()
    private var generation=0L
    private val mutable=MutableStateFlow(McpLifetimeState())
    val state:StateFlow<McpLifetimeState> = mutable
    private val cycles=McpServiceCycles(create)
    private var stopping=false
    suspend fun start(){
        val epoch=synchronized(lock){if(mutable.value.closed||stopping)throw McpRefused();mutable.value=mutable.value.copy(starting=true,issue=null);generation}
        try{
            val service=cycles.start()
            synchronized(lock){if(generation==epoch&&!mutable.value.closed)mutable.value=McpLifetimeState(owner=service.coordinator)}
        }catch(error:CancellationException){throw error}
        catch(error:Exception){synchronized(lock){if(generation==epoch&&!mutable.value.closed)mutable.value=McpLifetimeState(
            failureStage=(error as? McpRefused)?.stage?:McpFailureStage.unknown,
            issue="The packaged MCP service is unavailable or its runtime inventory failed verification. Rebuild the complete MCP package; no alternate listener was started.")}}
        finally{synchronized(lock){if(generation==epoch&&!mutable.value.closed)mutable.value=mutable.value.copy(starting=false)}}
    }
    suspend fun stop(){withContext(NonCancellable){
        synchronized(lock){generation++;stopping=true;mutable.value=mutable.value.copy(starting=true,issue=null)}
        try{cycles.stop();synchronized(lock){stopping=false;if(!mutable.value.closed)mutable.value=McpLifetimeState(issue="Local service stopped. All grants were revoked; a restart requires new explicit grants.")}}
        catch(error:Throwable){synchronized(lock){mutable.value=mutable.value.copy(starting=false,closed=true,issue="Service shutdown could not be confirmed. Restart the application before starting another owner.")};throw error}
    }}
    suspend fun close(){withContext(NonCancellable){synchronized(lock){generation++;stopping=true;mutable.value=mutable.value.copy(starting=false,closed=true)};try{cycles.close();synchronized(lock){mutable.value=McpLifetimeState(closed=true)}}catch(error:Throwable){synchronized(lock){mutable.value=mutable.value.copy(issue="Service shutdown could not be confirmed. Capture status is unknown.")};throw error}}}
}

private suspend fun openMcpDesktopService(privateParent:Path,helper:Path,helperHash:String,core:WorkbenchCore,device:String):McpDesktopServiceOwner {
    return run {
        val held=AtomicReference<DesktopMcpBundle?>(null)
        var coordinator:DesktopMcpCoordinator?=null
        try{
            val bundle=withContext(Dispatchers.IO){DesktopMcpBundle.prepare(privateParent).also{held.set(it)}}
            val owner=DesktopMcpService.open(bundle.runtime,privateParent,helper,helperHash,core,device);coordinator=owner
            McpDesktopOwned(owner,bundle).also{held.set(null);coordinator=null}
        }finally{withContext(NonCancellable){try{coordinator?.close()}finally{held.getAndSet(null)?.close()}}}
    }
}
