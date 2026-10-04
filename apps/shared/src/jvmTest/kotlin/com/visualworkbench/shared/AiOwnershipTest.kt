package com.visualworkbench.shared
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.*
import org.junit.Test
class AiOwnershipTest {
    @Test fun cancelledProducerRetainsSlotUntilLateHandleIsClosed()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val slots=Semaphore(1)
        val entered=CompletableDeferred<Unit>();val finish=CompletableDeferred<Unit>();val closeEntered=CompletableDeferred<Unit>();val closeFinish=CompletableDeferred<Unit>();val released=AtomicInteger();val destroyed=AtomicInteger();val signaled=CompletableDeferred<Unit>()
        try{val caller=launch { ownAiResult(scope,slots,{Unit},{signaled.complete(Unit)},{destroyed.incrementAndGet()},{_:String->closeEntered.complete(Unit);closeFinish.await();released.incrementAndGet()}){entered.complete(Unit);finish.await();"late-handle"} }
            withTimeout(5000){entered.await()};caller.cancel();withTimeout(5000){signaled.await()};assertEquals(0,slots.availablePermits);assertEquals(0,destroyed.get());finish.complete(Unit);withTimeout(5000){closeEntered.await()};assertEquals(0,slots.availablePermits);closeFinish.complete(Unit);withTimeout(5000){caller.join()};assertEquals(1,released.get());assertEquals(1,destroyed.get());assertEquals(1,slots.availablePermits)
        }finally{finish.complete(Unit);closeFinish.complete(Unit);scope.cancel()}
    }
    @Test fun deliveredHandleIsTransferredExactlyOnce()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val slots=Semaphore(1);val released=AtomicInteger();val destroyed=AtomicInteger()
        try{val result=ownAiResult(scope,slots,{Unit},{},{destroyed.incrementAndGet()},{_:String->released.incrementAndGet()}){"owned"};assertEquals("owned",result);assertEquals(0,released.get());assertEquals(1,destroyed.get());assertEquals(1,slots.availablePermits)}finally{scope.cancel()}
    }
    @Test fun failedProducerDestroysTokenAndRestoresCapacity()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val slots=Semaphore(1);val destroyed=AtomicInteger()
        try{assertAiFailure<AiFailure>{ownAiResult<Unit,String>(scope,slots,{Unit},{},{destroyed.incrementAndGet()},{}){throw AiFailure(AiFailureKind.Proof)}};assertEquals(1,destroyed.get());assertEquals(1,slots.availablePermits)}finally{scope.cancel()}
    }
}

private inline fun <reified T:Throwable> assertAiFailure(block:()->Unit):T {
    try { block() } catch(error:Throwable) { if(error is T)return error;throw AssertionError("Unexpected exception type",error) }
    throw AssertionError("Expected exception was not thrown")
}
