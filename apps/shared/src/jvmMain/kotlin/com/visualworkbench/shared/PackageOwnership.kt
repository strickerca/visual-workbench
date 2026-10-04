package com.visualworkbench.shared
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
/** JNI-free settlement boundary for deterministic lost-delivery tests. The
 * supervisor scope belongs to the adapter, never to a cancellable caller. */
internal suspend fun <C,T> ownPackageResult(scope:CoroutineScope,slots:Semaphore,create:()->C,
    signal:(C)->Unit,destroy:(C)->Unit,release:suspend(T)->Unit,block:suspend(C)->T):T {
    currentCoroutineContext().ensureActive()
    if(!slots.tryAcquire())throw PackageFailure(PackageFailureKind.Busy)
    try {
        currentCoroutineContext().ensureActive()
        val token=create()
        try {
            val owned=AtomicReference<T?>(null)
            val producer=scope.async { block(token).also { owned.set(it) } }
            try {
                val value=producer.await()
                currentCoroutineContext().ensureActive()
                owned.set(null)
                return value
            } catch(error:CancellationException) {
                try { signal(token) } catch(cleanup:Exception) { error.addSuppressed(cleanup) }
                withContext(NonCancellable) {
                    try { producer.await() } catch(_:Exception) {}
                    try { owned.getAndSet(null)?.let { release(it) } } catch(cleanup:Exception) { error.addSuppressed(cleanup) }
                }
                throw error
            }
        } finally { destroy(token) }
    } finally { slots.release() }
}

/** Every caller observes the same settled shutdown, including concurrent or
 * cancelled waiters. Setting the admission fence does not imply completion. */
internal class JoinedPackageClose {
    private val closing=AtomicBoolean(false)
    private val gate=Mutex()
    private var settled=false
    private var failure:Throwable?=null
    fun begin(){closing.set(true)}
    fun check(){if(closing.get())throw PackageFailure(PackageFailureKind.Closed)}
    suspend fun close(action:suspend()->Unit) {
        begin()
        withContext(NonCancellable) {
            gate.withLock {
                if(!settled){try{action()}catch(error:Throwable){failure=error}finally{settled=true}}
                failure?.let{throw it}
            }
        }
    }
}
