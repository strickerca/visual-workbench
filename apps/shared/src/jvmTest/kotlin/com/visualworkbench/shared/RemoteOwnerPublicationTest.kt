package com.visualworkbench.shared

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.*
import org.junit.Test

class RemoteOwnerPublicationTest {
    private class Owner(var retired:Boolean=false,var attempts:Int=0)
    @Test fun closeActuallyBlocksAtTheConstructionPublicationMonitorAndCapturesThatOwner() {
        val closed=AtomicBoolean(false);val gate=RemoteOwnerPublication<Owner>(closed){it.retired}
        val entered=CountDownLatch(1);val publish=CountDownLatch(1);val closeStarted=CountDownLatch(1)
        val opened=AtomicReference<Owner?>();val captured=AtomicReference<Owner?>();val error=AtomicReference<Throwable?>()
        val opener=Thread({try{opened.set(gate.acquire({}){entered.countDown();check(publish.await(5,TimeUnit.SECONDS));Owner()})}catch(t:Throwable){error.set(t)}},"owned-remote-construction")
        val closer=Thread({try{closeStarted.countDown();captured.set(gate.seal())}catch(t:Throwable){error.set(t)}},"owned-remote-seal")
        try {
            opener.start();assertTrue(entered.await(5,TimeUnit.SECONDS));closer.start();assertTrue(closeStarted.await(5,TimeUnit.SECONDS))
            val deadline=System.nanoTime()+5_000_000_000L
            while(closer.isAlive&&closer.state!=Thread.State.BLOCKED&&System.nanoTime()<deadline)Thread.yield()
            // Actual JVM monitor contention, not a sleep-based guess of a race.
            assertEquals(Thread.State.BLOCKED,closer.state);assertFalse(closed.get());assertNull(captured.get())
            publish.countDown();opener.join(5_000);closer.join(5_000)
            assertFalse(opener.isAlive);assertFalse(closer.isAlive);assertNull(error.get())
            assertNotNull(opened.get());assertSame(opened.get(),captured.get());assertTrue(closed.get())
        }finally{publish.countDown();if(opener.isAlive)opener.join(5_000);if(closer.isAlive)closer.join(5_000);assertFalse(opener.isAlive);assertFalse(closer.isAlive)}
    }
    @Test fun sealBeforeConstructionRefusesWithoutPublishingOrRunningFactory() {
        val gate=RemoteOwnerPublication<Owner>(AtomicBoolean(false)){it.retired};assertNull(gate.seal())
        var called=false;try{gate.acquire({}){called=true;Owner()};fail("Closed owner was constructed")}
        catch(error:SessionFailure){assertEquals(SessionFailureKind.Closed,error.kind)}
        assertFalse(called);assertNull(gate.seal())
    }
    @Test fun onlyAnActuallyRetiredOwnerCanBeReplacedWhileTheLinkIsOpen() {
        val gate=RemoteOwnerPublication<Owner>(AtomicBoolean(false)){it.retired};val first=gate.acquire({}){Owner()}
        assertSame(first,gate.acquire({}){error("Active owner replaced")});first.retired=true
        val next=gate.acquire({}){Owner()};assertNotSame(first,next);assertSame(next,gate.seal())
    }
    @Test fun pendingRetirementRetainsTheExactOwnerAcrossSealRetries() {
        val gate=RemoteOwnerPublication<Owner>(AtomicBoolean(false)){it.retired};val owner=gate.acquire({}){Owner()}
        val captured=requireNotNull(gate.seal());captured.attempts++
        val pending=SessionFailure(SessionFailureKind.RemoteRetirementPending)
        try{throw pending}catch(caught:SessionFailure){assertSame(pending,caught)}
        assertSame(owner,gate.seal());assertFalse(owner.retired)
        requireNotNull(gate.seal()).also{it.attempts++;it.retired=true}
        assertEquals(2,owner.attempts);assertSame(owner,gate.seal())
    }
    @Test fun failedConstructorNeverLeavesAPublishedOwner() {
        val gate=RemoteOwnerPublication<Owner>(AtomicBoolean(false)){it.retired};val original=SessionFailure(SessionFailureKind.Worker)
        try{gate.acquire({}){throw original};fail("Constructor failure lost")}
        catch(caught:SessionFailure){assertSame(original,caught)}
        assertNull(gate.seal())
    }
    @Test fun parentRefusalIsPreservedBeforeCreatingAnyRemoteChild() {
        val gate=RemoteOwnerPublication<Owner>(AtomicBoolean(false)){it.retired};val original=SessionFailure(SessionFailureKind.Closed);var called=false
        try{gate.acquire({throw original}){called=true;Owner()};fail("Closed parent accepted")}
        catch(caught:SessionFailure){assertSame(original,caught)}
        assertFalse(called);assertNull(gate.seal())
    }
}
