package com.visualworkbench.android.editor

import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong

class ProjectCounterFloorTest {
    @Test fun observedDraftFloorPrecedesTheNextOrdinaryAllocationWithoutWastingARange(){
        val next=AtomicLong(1)
        observeOperationCounter(next,1_000_001,101uL)
        assertEquals(101L,next.getAndAdd(3))
        repeat(100){observeOperationCounter(next,1_000_001,101uL)}
        assertEquals(104L,next.get())
    }
    @Test fun concurrentFloorsCannotRewindAnExistingHigherCounter(){
        val next=AtomicLong(1);val executor=Executors.newFixedThreadPool(2)
        try{
            val work=(1L..128L).map{floor->executor.submit{observeOperationCounter(next,1000,floor.toULong())}}
            work.forEach{it.get()};assertEquals(128L,next.get())
            observeOperationCounter(next,1000,1uL);assertEquals(128L,next.get())
        }finally{executor.shutdownNow()}
    }
    @Test fun invalidOrUnreservedFloorsNeverChangeTheAllocator(){
        val next=AtomicLong(10)
        for(floor in listOf(0uL,100uL,ULong.MAX_VALUE)){
            assertTrue(runCatching{observeOperationCounter(next,100,floor)}.isFailure)
            assertEquals(10L,next.get())
        }
    }
}
