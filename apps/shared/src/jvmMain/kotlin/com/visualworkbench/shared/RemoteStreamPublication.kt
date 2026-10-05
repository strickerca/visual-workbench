package com.visualworkbench.shared

import java.util.concurrent.atomic.AtomicLong

/** Fixed key set, numeric-only aggregate observations. Never supplies authority. */
internal class RemoteStreamCounters(private vararg val names:String) {
    private val values=Array(names.size){AtomicLong()}
    fun increment(name:String){val i=names.indexOf(name);require(i>=0);values[i].updateAndGet{if(it==Long.MAX_VALUE)it else it+1}}
    fun snapshot():Map<String,ULong> = names.indices.associate{names[it] to values[it].get().toULong()}
}
/** Serialize the watcher and frame-side snapshot by the native monotonic revision.
 * An older watcher response cannot overwrite a newer revocation/source snapshot. */
internal class RemoteStreamPublication(initial:RemoteEditState,private val publish:(RemoteEditState)->Unit) {
    private var current=initial
    @Synchronized fun offer(next:RemoteEditState):Boolean {
        if(next.revision<=current.revision)return false
        current=next;publish(next);return true
    }
}
public fun remoteEditStreamDiagnostics(remote:WorkbenchRemoteEdit):Map<String,ULong> =
    (remote as? NativeRemoteEdit)?.streamDiagnostics() ?: emptyMap()
