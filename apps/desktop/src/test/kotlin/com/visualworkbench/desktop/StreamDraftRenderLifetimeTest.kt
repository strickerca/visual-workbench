package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import androidx.compose.ui.input.pointer.PointerType
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class StreamDraftRenderLifetimeTest {
    @Test fun firstContactRequiresPenOrPrimaryMouseAndRejectsPalmAndAuxiliaryButtons(){
        assertTrue(streamContactToolAllowed(PointerType.Stylus,false,false,false))
        assertTrue(streamContactToolAllowed(PointerType.Mouse,true,false,false))
        assertFalse(streamContactToolAllowed(PointerType.Mouse,false,false,false))
        assertFalse(streamContactToolAllowed(PointerType.Mouse,true,true,false))
        assertFalse(streamContactToolAllowed(PointerType.Stylus,false,true,false))
        assertFalse(streamContactToolAllowed(PointerType.Touch,true,false,false))
    }
    @Test fun actualCancelledPreparationMustSettleBeforeRetirementReturns()=runBlocking{
        val lifetime=StreamDraftRenderLifetime()
        val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        val rendering=launch{
            assertTrue(lifetime.register(checkNotNull(currentCoroutineContext()[Job])))
            entered.complete(Unit)
            try{awaitCancellation()}finally{withContext(NonCancellable){release.await()}}
        }
        entered.await()
        val closing=async(start=CoroutineStart.UNDISPATCHED){lifetime.close()}
        assertFalse(lifetime.admitting)
        assertFalse(closing.isCompleted)
        assertFalse(lifetime.register(Job().also{it.complete()}))
        release.complete(Unit);closing.await();rendering.join()
        assertTrue(rendering.isCompleted)
        lifetime.close()
    }
    @Test fun preparationCannotJoinItsOwnRetirement()=runBlocking{
        val lifetime=StreamDraftRenderLifetime()
        val worker=async{
            lifetime.register(checkNotNull(currentCoroutineContext()[Job]))
            assertTrue(runCatching{lifetime.close()}.exceptionOrNull() is IllegalStateException)
        }
        worker.await();lifetime.close()
    }
    @Test fun streamSavePlansReserveWholeRangesWithoutRewindingOrOverflow(){
        val counters=DesktopStreamSaveCounters()
        assertEquals(1uL,counters.reserve(1uL))
        assertEquals(4097uL,counters.reserve(1uL))
        assertEquals(50_000uL,counters.reserve(50_000uL))
        assertTrue(runCatching{counters.reserve(ULong.MAX_VALUE)}.isFailure)
        assertEquals(54_096uL,counters.reserve(2uL))
    }
    @Test fun physicalOverlayAndAwtCameraApplyDensityExactlyOnce(){
        val frame=CaptureStreamFrame("synthetic",1uL,1uL,400u,200u,0,0,1.0,1u,1,1,"windows")
        val physical=streamContactCamera(frame,1000.0,800.0)
        assertEquals(Camera(Point(200.0,100.0),2.5,0.0,1000.0,800.0),physical)
        assertEquals(physical.copy(scale=1.25,viewportWidth=500.0,viewportHeight=400.0),streamAwtCamera(physical,2.0))
        assertTrue(runCatching{streamAwtCamera(physical,0.0)}.isFailure)
    }
}
