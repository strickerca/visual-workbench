package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import com.visualworkbench.bindings.core.CoreException
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import kotlin.coroutines.resumeWithException

/** Resource-producing calls run independently until their handle is either
 * delivered or disposed. Cancelling the caller signals Rust but never cancels
 * the producer before it can release an already-created native handle. */
private val transferJob = SupervisorJob()
private val transferScope = CoroutineScope(transferJob + Dispatchers.Default)
private val transferSlots = Semaphore(8)
private val cleanupFailure = AtomicReference<Throwable?>(null)
private fun cleanupFailed(error:Throwable) { cleanupFailure.compareAndSet(null,error) }
private fun checkCleanup() { cleanupFailure.get()?.let { throw CoreFailure(CoreFailureKind.Worker,it) } }

/** Preserve the caller dispatcher while entering NonCancellable, so returning
 * from the native worker cannot skip the rest of a cancelled caller's finally. */
internal suspend fun <T> releaseNative(block:suspend ()->T):T = withContext(NonCancellable) {
    withContext(Dispatchers.Default) { block() }
}

private class CancellationOwner {
    val value = Cancellation()
    private var live = true
    @Synchronized fun cancel() { if(live) value.cancel() }
    @Synchronized fun dispose() { if(live) { live=false; value.destroy() } }
}
internal suspend fun <T:Any,R> acquireNative(create:suspend (Cancellation)->T,wrap:(T)->R,release:suspend (T)->Unit):R = suspendCancellableCoroutine { continuation ->
    try { checkCleanup() } catch(error:Throwable) { continuation.resumeWithException(error);return@suspendCancellableCoroutine }
    if(!transferSlots.tryAcquire()) { continuation.resumeWithException(CoreFailure(CoreFailureKind.Backpressure)); return@suspendCancellableCoroutine }
    transferScope.launch {
        var owned:T?=null
        var transferred=false
        var cancellation:CancellationOwner?=null
        try {
            if(!continuation.isActive)return@launch
            val token=CancellationOwner();cancellation=token
            continuation.invokeOnCancellation { try { token.cancel() } catch(error:Throwable) { cleanupFailed(error) } }
            val handle=create(token.value);owned=handle
            val wrapped=wrap(handle)
            // The onCancellation handler also covers cancellation after resume
            // but before the original dispatcher delivers the result.
            continuation.resume(wrapped,onCancellation={_,_,_->transferScope.launch { try { release(handle) } catch(error:Throwable) { cleanupFailed(error) } }})
            transferred=true
        } catch(error:Throwable) {
            if(continuation.isActive)continuation.resumeWithException(when(error){is CoreException->mapFailure(error);is com.visualworkbench.bindings.core.SessionException->sessionFailure(error);else->error})
        } finally {
            try { if(!transferred)owned?.let { release(it) } } catch(error:Throwable) { cleanupFailed(error) }
            finally {
                try { cancellation?.dispose() } catch(error:Throwable) { cleanupFailed(error) }
                finally { transferSlots.release() }
            }
        }
    }
}
internal suspend fun drainNativeTransfers() { withTimeout(30_000) { while(transferJob.children.any()) { transferJob.children.toList().joinAll() } };checkCleanup() }
