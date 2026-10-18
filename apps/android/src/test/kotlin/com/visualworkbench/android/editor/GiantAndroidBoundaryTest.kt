package com.visualworkbench.android.editor

import com.visualworkbench.shared.GiantViewportSession
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class GiantAndroidBoundaryTest {
    private fun refuses(block: () -> Unit) {
        try { block(); fail("invalid storage admitted") } catch (_: IllegalArgumentException) { }
    }
    @Test fun physicalSurfaceHasAxisAreaAndStrideHeadroom() {
        admitGiantAndroidSurface(2048, 2560)
        admitGiantAndroidSurface(8192, 640)
        assertEquals(96uL * 1024uL * 1024uL, GIANT_ANDROID_SURFACE_BYTES)
        assertTrue(5uL * 1024uL * 1024uL * 4uL * 4uL < GIANT_ANDROID_SURFACE_BYTES)
        for ((w,h) in listOf(2049 to 2560,8193 to 1,1 to 8193,0 to 1,1 to -1,Int.MAX_VALUE to Int.MAX_VALUE))
            refuses { admitGiantAndroidSurface(w,h) }
    }
    @Test fun bitmapAdmissionUsesActualStrideAllocationAndAllChargedCopies() {
        admitGiantAndroidBitmap(256,256,256*256*4,1024,256*256*4)
        admitGiantAndroidBitmap(256,256,256*256*4,1040,256*256*4+4096)
        admitGiantAndroidBitmap(1,1,4,8,8)
        assertTrue(GiantViewportSession.TILE_CHARGE > 3L*256*256*4)
    }
    @Test fun malformedOrOversizedBitmapStorageCannotBorrowTheSurfaceAllowance() {
        for (values in listOf(intArrayOf(257,1,1028,1028,1028),intArrayOf(1,1,3,4,4),
            intArrayOf(2,1,8,4,8),intArrayOf(256,256,262144,1040,262144),
            intArrayOf(1,1,4,Int.MAX_VALUE,Int.MAX_VALUE),intArrayOf(1,1,4,4,-1)))
            refuses { admitGiantAndroidBitmap(values[0],values[1],values[2],values[3],values[4]) }
    }
    @Test fun failedCreationIsOwnedUntilExplicitRetiredRelease() = runBlocking {
        val handoff = GiantAndroidBitmapHandoff<Any>(); val objectOwner = Any(); var releases = 0
        try { handoff.create(Dispatchers.Unconfined) { acquired -> acquired(objectOwner); error("synthetic conversion failure") }; fail("failure hidden") }
        catch (_: IllegalStateException) { }
        assertEquals(1,handoff.retainedCount); assertEquals(0,releases)
        handoff.releaseRetired { assertSame(objectOwner,it); releases++ }
        handoff.releaseRetired { releases++ }
        assertEquals(0,handoff.retainedCount); assertEquals(1,releases)
    }
    @Test fun releaseFailureRetainsTheSamePendingBitmapForRetry() = runBlocking {
        val handoff = GiantAndroidBitmapHandoff<Any>(); val objectOwner = Any()
        try { handoff.create(Dispatchers.Unconfined) { acquired -> acquired(objectOwner); error("synthetic") } }
        catch (_: IllegalStateException) { }
        try { handoff.releaseRetired { error("synthetic release failure") }; fail("failure hidden") }
        catch (_: IllegalStateException) { }
        assertEquals(1,handoff.retainedCount)
        handoff.releaseRetired { assertSame(objectOwner,it) }
        assertEquals(0,handoff.retainedCount)
    }
    @Test fun successfulHandoffTransfersExactlyOneOwner() = runBlocking {
        val handoff = GiantAndroidBitmapHandoff<Any>(); val objectOwner = Any()
        assertSame(objectOwner,handoff.create(Dispatchers.Unconfined) { acquired -> acquired(objectOwner); objectOwner })
        assertEquals(0,handoff.retainedCount)
        handoff.releaseRetired { fail("successful tile was double released") }
    }
    private class Queued : CoroutineDispatcher() {
        private val tasks = ArrayDeque<Runnable>()
        override fun dispatch(context: CoroutineContext, block: Runnable) { tasks.addLast(block) }
        fun next() { tasks.removeFirst().run() }
    }
    @Test fun promptCancellationAfterConversionKeepsTheUndeliveredOwner() = runBlocking {
        val caller = Queued(); val conversion = Queued()
        val handoff = GiantAndroidBitmapHandoff<Any>(); val objectOwner = Any()
        val task = launch(caller) {
            handoff.create(conversion) { acquired -> acquired(objectOwner); objectOwner }
            fail("cancelled creation adopted")
        }
        caller.next(); conversion.next() // Return to caller remains queued.
        task.cancel(); caller.next(); task.join()
        assertEquals(1,handoff.retainedCount)
        handoff.releaseRetired { assertSame(objectOwner,it) }
        assertEquals(0,handoff.retainedCount)
    }
}
