package com.visualworkbench.desktop

import kotlinx.coroutines.*

/** Compose preparation is outside the editor's own scope. Retain and join its
 * actual jobs before child/native retirement; disposing a composable is not a
 * completion receipt. All admission/draw calls run on the UI dispatcher. */
internal class StreamDraftRenderLifetime {
    private val jobs = linkedSetOf<Job>()
    @Volatile var admitting = true; private set
    fun register(job:Job):Boolean = synchronized(jobs){
        if(!admitting)return@synchronized false
        jobs+=job
        job.invokeOnCompletion{synchronized(jobs){jobs-=job}}
        true
    }
    fun seal(){synchronized(jobs){admitting=false}}
    suspend fun close(){
        val caller=currentCoroutineContext()[Job]
        seal()
        withContext(NonCancellable){
            val held=synchronized(jobs){jobs.toList()}
            check(caller !in held){"Retire the annotation canvas from outside its render jobs"}
            held.forEach{it.cancel()};held.joinAll()
        }
    }
}
