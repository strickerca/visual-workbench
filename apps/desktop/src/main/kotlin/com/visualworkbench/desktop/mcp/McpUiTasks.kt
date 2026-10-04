package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.*

/** Tracks package compilation/export producers launched from disposable panels.
 * Main seals and joins these before the coordinator or editor can be destroyed. */
internal class McpUiTasks {
    private val lock=Any()
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
    private val tasks=LinkedHashSet<Job>()
    private var closed=false
    private val stopped=CompletableDeferred<Unit>()
    suspend fun <T> run(body:suspend()->T):T {
        currentCoroutineContext().ensureActive()
        val task=synchronized(lock){
            if(closed || tasks.size>=2)throw McpRefused()
            scope.async(start=CoroutineStart.LAZY){body()}.also{job->tasks+=job;job.invokeOnCompletion{synchronized(lock){tasks.remove(job)}}}
        }
        task.start()
        try{return task.await()}catch(error:CancellationException){task.cancel();withContext(NonCancellable){task.join()};throw error}
    }
    suspend fun close(){withContext(NonCancellable){
        val first=synchronized(lock){if(closed)false else{closed=true;true}}
        if(first){try{val pending=synchronized(lock){tasks.toList()};pending.forEach{it.cancel()};pending.joinAll();stopped.complete(Unit)}catch(error:Throwable){stopped.completeExceptionally(error)}finally{scope.cancel()}}
        stopped.await()
    }}
}
