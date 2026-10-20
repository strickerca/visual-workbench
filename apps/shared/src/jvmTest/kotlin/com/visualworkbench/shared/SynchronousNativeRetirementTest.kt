package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException

class SynchronousNativeRetirementTest {
    @Test fun refusedDisposalRetainsHandleAndRetryDoesNotPretendSuccess(){
        var disposals=0;var destructions=0
        val retirement=SynchronousNativeRetirement({if(++disposals==1)error("synthetic disposal refusal")},{destructions++})
        try{retirement.close();fail("expected refusal")}catch(_:IllegalStateException){}
        assertEquals(0,destructions)
        try{retirement.check();fail("must seal new calls")}catch(error:WorkflowFailure){assertEquals(WorkflowFailureKind.Closed,error.kind)}
        retirement.close();retirement.close()
        assertEquals(2,disposals);assertEquals(1,destructions)
    }
    @Test fun uncertainDestructorFailureStaysVisibleWithoutCallingConsumedPointerAgain(){
        var disposals=0;var destructions=0;val failure=IllegalStateException("synthetic consumed pointer")
        val retirement=SynchronousNativeRetirement({disposals++},{destructions++;throw failure})
        repeat(2){try{retirement.close();fail("must retain failure")}catch(error:IllegalStateException){assertSame(failure,error)}}
        assertEquals(1,disposals);assertEquals(1,destructions)
    }
    @Test fun concurrentCloseCannotReportRetirementWhileDisposalIsStillRunning(){
        val entered=CountDownLatch(1);val release=CountDownLatch(1);val executor=Executors.newFixedThreadPool(2)
        val retirement=SynchronousNativeRetirement({entered.countDown();check(release.await(5,TimeUnit.SECONDS))},{})
        try{
            val first=executor.submit{retirement.close()};assertTrue(entered.await(5,TimeUnit.SECONDS))
            val second=executor.submit{retirement.close()}
            try{second.get(20,TimeUnit.MILLISECONDS);fail("close returned before disposal")}catch(_:TimeoutException){}
            release.countDown();first.get(5,TimeUnit.SECONDS);second.get(5,TimeUnit.SECONDS)
        }finally{release.countDown();executor.shutdownNow()}
    }
}
