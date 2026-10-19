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
    @Test fun captureSaveReservesTheWholeRangeAndOrdinaryEditsCannotReuseItsTail(){
        val next=AtomicLong(1)
        val saved=reserveOperationCounters(next,1_000_001,100uL,4096)
        assertEquals(100uL,saved)
        assertEquals(4196L,next.getAndAdd(3))
        assertEquals(4199uL,reserveOperationCounters(next,1_000_001,1uL,1024))
    }
    @Test fun concurrentCaptureRangesNeverOverlap(){
        val next=AtomicLong(1);val executor=Executors.newFixedThreadPool(2)
        try{
            val futures=(1..16).map{executor.submit<ULong>{reserveOperationCounters(next,1_000_001,1uL,4096)}}
            val starts=futures.map{it.get()}.sorted()
            assertEquals(16,starts.distinct().size)
            starts.zipWithNext().forEach{(a,b)->assertTrue(b>=a+4096uL)}
            assertEquals(1L+16*4096,next.get())
        }finally{executor.shutdownNow()}
    }
    @Test fun exhaustedCaptureRangeRefusesAtomicallyAtTheDurableBoundary(){
        val next=AtomicLong(10)
        for((floor,count) in listOf(10uL to 4096,ULong.MAX_VALUE to 1,10uL to 4097)){
            assertTrue(runCatching{reserveOperationCounters(next,4105,floor,count)}.isFailure)
            assertEquals(10L,next.get())
        }
        assertEquals(10uL,reserveOperationCounters(next,4106,10uL,4096))
        assertEquals(4106L,next.get())
        assertTrue(runCatching{reserveOperationCounters(next,4106,1uL,1)}.isFailure)
        assertEquals(4106L,next.get())
    }
}
