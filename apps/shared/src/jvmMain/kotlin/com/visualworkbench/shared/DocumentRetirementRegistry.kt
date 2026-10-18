package com.visualworkbench.shared

import java.util.IdentityHashMap
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Failed final disposal retains the actual app owner. This is cleanup-only;
 * no active document transport can be looked up or installed through it. */
public object DocumentRetirementRegistry {
    private val pending:IdentityHashMap<Any,suspend()->Unit> = IdentityHashMap()
    private val gate:Mutex=Mutex()
    @Synchronized public fun retain(owner:Any,close:suspend()->Unit):Unit { pending[owner]=close }
    @Synchronized private fun entries():List<Pair<Any,suspend()->Unit>> = pending.entries.map{it.key to it.value}
    @Synchronized private fun remove(owner:Any):Unit { pending.remove(owner) }
    public suspend fun drain():Unit = withContext(NonCancellable){
        gate.withLock {
            var failure:Exception?=null
            for((owner,close)in entries())try{close();remove(owner)}catch(error:Exception){failure=error}
            if(failure!=null)throw DocumentFailure(DocumentFailureKind.Isolation)
        }
    }
}
