package com.visualworkbench.shared
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import java.util.concurrent.atomic.AtomicReference
/** Kept JNI-free so cancellation races are exercised with deterministic gates. */
internal suspend fun <C,T> ownAiResult(scope:CoroutineScope,slots:Semaphore,create:()->C,signal:(C)->Unit,destroy:(C)->Unit,release:suspend(T)->Unit,block:suspend(C)->T):T{
    currentCoroutineContext().ensureActive();if(!slots.tryAcquire())throw AiFailure(AiFailureKind.Busy)
    try{currentCoroutineContext().ensureActive();val token=create();try{
        val owned=AtomicReference<T?>(null)
        val producer=scope.async{block(token).also{owned.set(it)}}
        try{val result=producer.await();currentCoroutineContext().ensureActive();owned.set(null);return result}
        catch(error:CancellationException){try{signal(token)}catch(e:Exception){error.addSuppressed(e)}
            withContext(NonCancellable){try{producer.await()}catch(_:Exception){};try{owned.getAndSet(null)?.let{release(it)}}catch(e:Exception){error.addSuppressed(e)}};throw error}
    }finally{destroy(token)}}finally{slots.release()}
}
