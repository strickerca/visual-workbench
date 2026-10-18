package com.visualworkbench.shared
import org.junit.Test
import org.junit.Assert.*
import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicInteger

class GiantOwnershipTest {
    @Test fun everyConcurrentAndCancelledCloserWaitsForActualRelease()=runBlocking {
        val owner=GiantHandleOwner();val entered=CompletableDeferred<Unit>();val finish=CompletableDeferred<Unit>();val released=AtomicInteger()
        val first=launch(start=CoroutineStart.UNDISPATCHED){owner.close{entered.complete(Unit);finish.await();released.incrementAndGet()}}
        entered.await();val second=launch(start=CoroutineStart.UNDISPATCHED){owner.close{error("duplicate release")}}
        try {first.cancel();assertFalse(first.isCompleted);assertFalse(second.isCompleted);assertEquals(0,released.get());assertFailsClosed{owner.checkOpen()}}
        finally{finish.complete(Unit);withContext(NonCancellable){first.join();second.join()}}
        owner.close{error("duplicate release")};assertEquals(1,released.get())
    }
    @Test fun releaseFailureIsSettledOnceAndRepeatedExactly()=runBlocking {
        val owner=GiantHandleOwner();val calls=AtomicInteger();val failure=IllegalStateException("synthetic failure")
        repeat(2){try{owner.close{calls.incrementAndGet();throw failure};fail("expected")}catch(error:IllegalStateException){assertSame(failure,error)}}
        assertEquals(1,calls.get());assertFailsClosed{owner.checkOpen()}
    }
    private fun assertFailsClosed(block:()->Unit){try{block();fail("expected closed")}catch(error:CoreFailure){assertEquals(CoreFailureKind.Closed,error.kind)}}
}
