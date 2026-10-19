package com.visualworkbench.android.editor

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Main-confined. Detached surfaces remain owned until their real renderer
 * release callback confirms retirement. A successor cannot allocate a second
 * pipeline against the first surface's single reservation. */
internal class CanvasRetirementOwner {
    private val retained=linkedSetOf<suspend()->Unit>()
    private var current:(suspend()->Unit)?=null
    private var wake:(()->Unit)?=null
    private val gate=Mutex()
    private var retiring=false
    private var failure:Throwable?=null
    fun attach(hook:suspend()->Unit,ready:()->Unit){retained+=hook;current=hook;wake=ready}
    fun detach(hook:suspend()->Unit):Boolean {
        if(current!==hook)return false
        current=null;wake=null;return true
    }
    fun canCreate(hook:suspend()->Unit):Boolean = !retiring&&failure==null&&current===hook&&retained.firstOrNull()===hook
    fun confirmed(hook:suspend()->Unit){if(current!==hook)retained.remove(hook);wake?.invoke()}
    suspend fun retireAll():Unit=gate.withLock {
        failure?.let{throw it}
        retiring=true
        try {
            val joined=mutableSetOf<suspend()->Unit>()
            // Include surfaces attached while an earlier release was suspended.
            while(true){val hook=retained.firstOrNull{it !in joined}?:break;hook();joined+=hook}
        } catch(error:Throwable){failure=error;throw error}
        finally{retiring=false;wake?.invoke()}
    }
    internal fun retainedCount():Int=retained.size
}
