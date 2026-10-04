package com.visualworkbench.shared
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import org.junit.Test
import org.junit.Assert.*
import java.util.concurrent.atomic.AtomicInteger

class PackageOwnershipTest {
    @Test fun cancellationWaitsForProducerAndReleasesAnUndeliveredHandle():Unit=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val slots=Semaphore(1)
        val entered=CompletableDeferred<Unit>();val finish=CompletableDeferred<Unit>();val signal=CompletableDeferred<Unit>()
        val released=AtomicInteger();val destroyed=AtomicInteger()
        val job=launch { ownPackageResult(scope,slots,{Unit},{signal.complete(Unit)},{destroyed.incrementAndGet()},{_:Any->released.incrementAndGet()}){entered.complete(Unit);finish.await();Any()} }
        try { withTimeout(5000){entered.await()};job.cancel();withTimeout(5000){signal.await()};assertFalse(job.isCompleted);assertEquals(0,slots.availablePermits);assertEquals(0,destroyed.get());finish.complete(Unit);withTimeout(5000){job.join()};assertEquals(1,released.get());assertEquals(1,destroyed.get());assertEquals(1,slots.availablePermits) }
        finally{finish.complete(Unit);job.cancelAndJoin();scope.cancel()}
    }
    @Test fun aFailedOperationDoesNotCancelCallerAndSlotIsRecovered():Unit=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val slots=Semaphore(1);val destroyed=AtomicInteger()
        try {try{ownPackageResult<Unit,Unit>(scope,slots,{Unit},{},{destroyed.incrementAndGet()},{}){throw PackageFailure(PackageFailureKind.Integrity)};fail("expected typed failure")}catch(e:PackageFailure){assertEquals(PackageFailureKind.Integrity,e.kind)}
            assertTrue(currentCoroutineContext().isActive);assertEquals(1,slots.availablePermits);assertEquals(1,destroyed.get())
            assertEquals(7,ownPackageResult(scope,slots,{Unit},{},{},{}){7})
        }finally{scope.cancel()}
    }
    @Test fun exhaustedSlotsDoNotCreateNativeTokens():Unit=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default);val slots=Semaphore(1);slots.acquire();val created=AtomicInteger()
        try {try{ownPackageResult(scope,slots,{created.incrementAndGet()},{},{},{}){Unit};fail("expected busy")}catch(e:PackageFailure){assertEquals(PackageFailureKind.Busy,e.kind)};assertEquals(0,created.get())}finally{slots.release();scope.cancel()}
    }
    @Test fun concurrentAndCancelledCloseWaitForTheSameWorkerSettlement():Unit=runBlocking {
        val closer=JoinedPackageClose();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();val actions=AtomicInteger()
        val first=launch { closer.close {actions.incrementAndGet();entered.complete(Unit);release.await()} }
        var second:Job?=null
        try {
            withTimeout(5000){entered.await()}
            val waiting=launch(start=CoroutineStart.UNDISPATCHED){closer.close{fail("shutdown must run once")}};second=waiting
            waiting.cancel();yield();assertFalse(first.isCompleted);assertFalse(waiting.isCompleted)
            try{closer.check();fail("close must stop ordinary admission")}catch(error:PackageFailure){assertEquals(PackageFailureKind.Closed,error.kind)}
            release.complete(Unit);withTimeout(5000){joinAll(first,waiting)};assertEquals(1,actions.get());closer.close{fail("settled retry must not rerun")}
        }finally{release.complete(Unit);withContext(NonCancellable){first.cancelAndJoin();second?.cancelAndJoin()}}

    }

}
