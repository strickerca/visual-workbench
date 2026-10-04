package com.visualworkbench.android.capture

import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

class CaptureOwnerTest {
    @Test fun heldInitializationCannotPublishAfterEditorClose(){
        val released=AtomicInteger();val published=AtomicInteger();val error=AtomicReference<Throwable?>()
        val owner=CaptureOwner<String>{released.incrementAndGet()};val entered=CountDownLatch(1);val proceed=CountDownLatch(1)
        val worker=Thread{try{entered.countDown();check(proceed.await(2,TimeUnit.SECONDS));assertFalse(owner.install("late",{true}){published.incrementAndGet();true})}catch(value:Throwable){error.set(value)}}
        worker.start();try{assertTrue(entered.await(2,TimeUnit.SECONDS));owner.close();proceed.countDown();worker.join(2000);assertFalse(worker.isAlive);error.get()?.let{throw it};assertEquals(0,published.get());assertEquals(1,released.get())}
        finally{proceed.countDown();worker.join(2000);owner.close()}
    }
    @Test fun rejectedAndAcceptedOwnersAreReleasedExactlyOnce(){
        val values=mutableListOf<String>();val owner=CaptureOwner<String>{values+=it}
        assertFalse(owner.install("disposing",{false}){fail("inactive editor published");true})
        assertFalse(owner.install("other-owner",{true}){false})
        assertTrue(owner.install("owned",{true}){true});owner.close();owner.close()
        assertEquals(listOf("disposing","other-owner","owned"),values)
    }
}
