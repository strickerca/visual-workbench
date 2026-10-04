package com.visualworkbench.android.capture

/** A delayed initializer cannot publish a resource after its editor has closed.
 * Installation/publication and ownership transfer are one critical section;
 * release happens outside the gate and exactly once. */
internal class CaptureOwner<T>(private val release:(T)->Unit) {
    private val gate=Any()
    private var closed=false
    private var owned:T?=null
    fun install(value:T,alive:()->Boolean,publish:(T)->Boolean):Boolean {
        var accepted=false
        try {
            synchronized(gate){if(!closed&&owned==null&&alive()&&publish(value)){owned=value;accepted=true}}
            return accepted
        }finally{if(!accepted)release(value)}
    }
    fun close(){val value=synchronized(gate){closed=true;owned.also{owned=null}};value?.let(release)}
}
