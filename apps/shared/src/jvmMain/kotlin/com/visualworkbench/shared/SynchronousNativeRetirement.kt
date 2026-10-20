package com.visualworkbench.shared

import java.util.concurrent.atomic.AtomicBoolean

/** Retain a generated handle when native disposal refuses. A generated
 * destructor may consume its pointer before throwing, so that failure is
 * sticky: retry must keep reporting it without crossing the ABI again. */
internal class SynchronousNativeRetirement(private val dispose:()->Unit,private val destroy:()->Unit) {
    private val closing=AtomicBoolean(false)
    private var disposed=false
    private var destroyed=false
    private var destructionFailure:Throwable?=null
    fun check(){if(closing.get())throw WorkflowFailure(WorkflowFailureKind.Closed)}
    @Synchronized fun close(){
        closing.set(true)
        destructionFailure?.let{throw it}
        if(destroyed)return
        if(!disposed){dispose();disposed=true}
        try{destroy()}catch(error:Throwable){destructionFailure=error;throw error}
        destroyed=true
    }
}
