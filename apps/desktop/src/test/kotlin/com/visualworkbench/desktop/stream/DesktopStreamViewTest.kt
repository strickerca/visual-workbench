package com.visualworkbench.desktop.stream

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.*
import org.junit.Test
import javax.swing.JPanel
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class DesktopStreamViewTest {
    private val frame=CaptureStreamFrame("capture",7u,2u,2u,2u,14,29,1.25,3u,900L,1000L,"windows")
    private val live=CaptureStreamSignal(1u,5u,2u,7u,CaptureStreamStage.Live,2u,2u,0u,null)
    private class Charge:SessionResourceCharge {var closes=0;override fun close(){closes++}}
    private class Resources:SessionResources {
        val charges=mutableListOf<Charge>()
        override suspend fun reserveForeign(bytes:ULong):SessionResourceCharge=Charge().also{charges+=it}
    }
    private class Draw(override val frame:CaptureStreamFrame):CaptureStreamDraw {
        val receipts=mutableListOf<Boolean>();@Volatile var closes=0
        var onRows:suspend ()->Unit={}
        override suspend fun rgbaRows(firstRow:UInt,rowCount:UInt):ByteArray {
            onRows()
            return byteArrayOf(1,2,3,-1,4,5,6,-1,7,8,9,-1,10,11,12,-1)
        }
        override suspend fun presented(actuallyShown:Boolean){receipts+=actuallyShown}
        override suspend fun close(){closes++}
    }
    private class Stream(start:CaptureStreamSignal):WorkbenchCaptureStream {
        private val mutable=MutableStateFlow(start)
        override val signals=mutable
        val draws=ArrayDeque<Draw>()
        var background:CaptureFrozenBackground?=null
        var closes=0
        override fun signal()=mutable.value
        fun advance(value:CaptureStreamSignal){mutable.value=value}
        override suspend fun beginDraw():CaptureStreamDraw?=if(draws.isEmpty())null else draws.removeFirst()
        override suspend fun frozenBackground():CaptureFrozenBackground?=background
        override suspend fun firstContact(draftId:String):CaptureFreezeReceipt?=error("Caller owns freeze")
        override suspend fun resumeAfterDraftDisposed():String?=error("Caller owns resume")
        override suspend fun close(){closes++}
    }
    private class Backend(var autoPaint:Boolean):DesktopStreamBackend {
        override val container=JPanel()
        val offered=CompletableDeferred<Unit>()
        var pending:StreamCanvasSlot?=null
        var positive:StreamCanvasSlot?=null
        var retired=false
        var failRetire=false
        var offerEntered:CountDownLatch?=null
        var offerRelease:CountDownLatch?=null
        var retireEntered:CountDownLatch?=null
        var retireRelease:CountDownLatch?=null
        var boundCamera:Camera?=null
        override fun setCamera(value:Camera?){boundCamera=value}
        override fun mapPointer(point:Point)=point
        override fun surfaceBytes()=16L
        override fun canStartDraw()=!retired&&pending==null
        override fun offer(slot:StreamCanvasSlot):Boolean {
            pending=slot;offered.complete(Unit)
            if(autoPaint){slot.permit?.paint{};slot.losslessPaint?.complete(Unit)}
            offerEntered?.countDown();offerRelease?.await(5,TimeUnit.SECONDS)
            return true
        }
        override fun commit(slot:StreamCanvasSlot){check(pending===slot);positive?.raster?.close();positive=slot;pending=null}
        override fun rollback(slot:StreamCanvasSlot,offered:Boolean){if(pending===slot)pending=null;slot.raster.close()}
        override fun retire(){retireEntered?.countDown();retireRelease?.await(5,TimeUnit.SECONDS);retired=true;if(failRetire)error("retained platform surface");pending?.raster?.close();positive?.raster?.close();pending=null;positive=null}
    }
    private val core=object:WorkbenchCore {
        override fun newId(unixMs:ULong)=error("Unused")
        override fun newDeviceId()=error("Unused")
        override suspend fun create(options:CreateProject):WorkbenchProject=error("Unused")
        override suspend fun open(path:String):WorkbenchProject=error("Unused")
        override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("Unused")
        override fun cameraMatrix(camera:Camera,inverse:Boolean)=Transform()
        override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>)=points
    }

    @Test fun acknowledgedPaintPrecedesContactAndViewCloseLeavesStreamToParent()=runBlocking {
        val draw=Draw(frame);val stream=Stream(live).apply{draws+=draw};val resources=Resources();val backend=Backend(true)
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val view=DesktopStreamView(scope,core,stream,{true},resources,Dispatchers.Unconfined,{_,_,_,_->backend})
        try {
            withTimeout(5000){while(view.state.value.displayed!=frame)yield()}
            val contact=view.stopForContact(Point(3.0,4.0))
            assertEquals(frame,contact?.frame)
            assertEquals(Point(3.0,4.0),contact?.point)
            assertEquals(listOf(true),draw.receipts)
            assertEquals(1,draw.closes)
        }finally{view.close();scope.cancel()}
        assertTrue(backend.retired)
        assertEquals(0,stream.closes)
        assertEquals(listOf(1),resources.charges.map{it.closes})
    }

    @Test fun contactBeforePaintRefusesOnlyTheUndrawnTicketAndKeepsNoFalseFrame()=runBlocking {
        val draw=Draw(frame);val stream=Stream(live).apply{draws+=draw};val backend=Backend(false)
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val view=DesktopStreamView(scope,core,stream,{true},Resources(),Dispatchers.Unconfined,{_,_,_,_->backend})
        try {
            withTimeout(5000){backend.offered.await()}
            assertNull(view.stopForContact(Point(0.0,0.0)))
            assertNull(view.stopLiveDraws())
            assertEquals(listOf(false),draw.receipts)
            assertNull(view.state.value.displayed)
        }finally{view.close();scope.cancel()}
    }

    @Test fun canceledReadJoinsNativeSettlementAndRetainsFailedRenderer()=runBlocking {
        val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        val draw=Draw(frame).apply{onRows={entered.complete(Unit);withContext(NonCancellable){release.await()}}}
        val stream=Stream(live).apply{draws+=draw};val resources=Resources();val backend=Backend(false)
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val view=DesktopStreamView(scope,core,stream,{true},resources,Dispatchers.Unconfined,{_,_,_,_->backend})
        try {
            withTimeout(5000){entered.await()}
            val closing=launch(start=CoroutineStart.UNDISPATCHED){view.close()}
            yield();assertFalse(closing.isCompleted)
            release.complete(Unit);closing.join()
            assertEquals(listOf(false),draw.receipts)
            assertEquals(1,draw.closes)
            assertEquals(listOf(1),resources.charges.map{it.closes})
        }finally{release.complete(Unit);scope.cancel()}
        assertEquals(0,stream.closes)

        val another=Stream(live);val bad=Backend(false).apply{failRetire=true}
        val badScope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val failed=DesktopStreamView(badScope,core,another,{true},Resources(),Dispatchers.Unconfined,{_,_,_,_->bad})
        assertTrue(runCatching{failed.close()}.isFailure)
        assertTrue(runCatching{failed.close()}.isFailure)
        assertEquals(0,another.closes)
        badScope.cancel()
    }

    @Test fun exactLosslessBackgroundReplacesOnlyPixelsAndKeepsDraftCameraAndPreviewOwnership()=runBlocking {
        val draw=Draw(frame);val stream=Stream(live).apply{draws+=draw};val backend=Backend(true);val resources=Resources()
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val view=DesktopStreamView(scope,core,stream,{true},resources,Dispatchers.Unconfined,{_,_,_,_->backend})
        try {
            withTimeout(5000){while(view.state.value.displayed!=frame)yield()}
            assertEquals(frame,view.stopLiveDraws())
            val camera=Camera(Point(1.0,1.0),2.0,0.0,100.0,100.0)
            view.setCamera(camera)
            var backgroundCloses=0
            stream.background=object:CaptureFrozenBackground {
                override val frame=this@DesktopStreamViewTest.frame
                override val draftId="draft"
                override val lossless=true
                override val originalAssetId="c".repeat(64)
                override suspend fun rgbaRows(firstRow:UInt,rowCount:UInt)=ByteArray(16){if(it%4==3)(-1).toByte() else 27.toByte()}
                override suspend fun close(){backgroundCloses++}
            }
            val draft=CaptureDraftInfo("draft","document",frame,"b".repeat(64),5u)
            val annotation=CaptureAnnotationState(CaptureFreezeReceipt("draft",frame.frameId,frame.captureSessionId,frame.sourceGeneration),draft,lossless=true)
            var refreshes=0
            assertTrue(view.adoptLossless({refreshes++},{annotation}))
            assertEquals(1,refreshes)
            assertEquals(1,backgroundCloses)
            assertEquals(camera,backend.boundCamera)
            assertEquals(frame,view.state.value.displayed)
            assertTrue(view.state.value.lossless)
            assertEquals(listOf(1,0),resources.charges.map{it.closes})
        }finally{view.close();scope.cancel()}
        assertEquals(listOf(1,1),resources.charges.map{it.closes})
        assertEquals(0,stream.closes)
    }

    @Test fun sourceGenerationChangeDuringRowsRejectsOldPixelsBeforePlatformPaint()=runBlocking {
        val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        val draw=Draw(frame).apply{onRows={entered.complete(Unit);release.await()}}
        val stream=Stream(live).apply{draws+=draw};val backend=Backend(true);val resources=Resources()
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val view=DesktopStreamView(scope,core,stream,{true},resources,Dispatchers.Unconfined,{_,_,_,_->backend})
        try {
            withTimeout(5000){entered.await()}
            stream.advance(live.copy(sequence=2u,sourceGeneration=3u))
            release.complete(Unit)
            withTimeout(5000){while(draw.closes==0)yield()}
            assertEquals(listOf(false),draw.receipts)
            assertNull(view.state.value.displayed)
            assertFalse(backend.offered.isCompleted)
            assertEquals(listOf(1),resources.charges.map{it.closes})
        }finally{release.complete(Unit);view.close();scope.cancel()}
    }

    @Test fun canceledOfferStillAcknowledgesTheExactSubmittedFrameAtContact()=runBlocking {
        val second=frame.copy(frameId=8u)
        val firstDraw=Draw(frame);val secondDraw=Draw(second)
        val stream=Stream(live).apply{draws+=firstDraw;draws+=secondDraw}
        val backend=Backend(true);val resources=Resources()
        val ui=Executors.newSingleThreadExecutor().asCoroutineDispatcher()
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val view=DesktopStreamView(scope,core,stream,{true},resources,ui,{_,_,_,_->backend})
        try {
            withTimeout(5000){while(view.state.value.displayed!=frame)yield()}
            backend.offerEntered=CountDownLatch(1);backend.offerRelease=CountDownLatch(1)
            stream.advance(live.copy(sequence=2u))
            assertTrue(backend.offerEntered!!.await(5,TimeUnit.SECONDS))
            val contact=async(Dispatchers.Default){view.stopForContact(Point(1.0,1.0))}
            yield();backend.offerRelease!!.countDown()
            assertEquals(second,withTimeout(5000){contact.await()}?.frame)
            assertEquals(listOf(true),secondDraw.receipts)
            assertEquals(second,backend.positive?.frame)
            assertEquals(1,secondDraw.closes)
        }finally{backend.offerRelease?.countDown();view.close();scope.cancel();ui.close()}
    }

    @Test fun simultaneousCloseCallersJoinTheSameRetirement()=runBlocking {
        val stream=Stream(live);val backend=Backend(false)
        backend.retireEntered=CountDownLatch(1);backend.retireRelease=CountDownLatch(1)
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val view=DesktopStreamView(scope,core,stream,{true},Resources(),Dispatchers.Unconfined,{_,_,_,_->backend})
        try {
            val first=async(Dispatchers.Default){view.close()}
            assertTrue(backend.retireEntered!!.await(5,TimeUnit.SECONDS))
            val second=async(Dispatchers.Default){view.close()}
            yield();assertFalse(second.isCompleted)
            backend.retireRelease!!.countDown()
            withTimeout(5000){first.await();second.await()}
            assertTrue(backend.retired)
        }finally{backend.retireRelease!!.countDown();scope.cancel()}
    }

    @Test fun losslessPaintAfterEpochSealCannotReplaceProvisionalFrame()=runBlocking {
        val stream=Stream(live).apply{draws+=Draw(frame)};val backend=Backend(true)
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val view=DesktopStreamView(scope,core,stream,{true},Resources(),Dispatchers.Unconfined,{_,_,_,_->backend})
        try {
            withTimeout(5000){while(view.state.value.displayed!=frame)yield()}
            view.stopLiveDraws();backend.autoPaint=false
            stream.background=object:CaptureFrozenBackground {
                override val frame=this@DesktopStreamViewTest.frame
                override val draftId="draft";override val lossless=true;override val originalAssetId="c".repeat(64)
                override suspend fun rgbaRows(firstRow:UInt,rowCount:UInt)=ByteArray(16){if(it%4==3)(-1).toByte() else 27.toByte()}
                override suspend fun close()=Unit
            }
            val draft=CaptureDraftInfo("draft","document",frame,"b".repeat(64),5u)
            val state=CaptureAnnotationState(CaptureFreezeReceipt("draft",frame.frameId,frame.captureSessionId,
                frame.sourceGeneration),draft,lossless=true)
            val adopting=async(Dispatchers.Default){view.adoptLossless({},{state})}
            withTimeout(5000){while(backend.pending?.losslessPaint==null)yield()}
            stream.advance(live.copy(connectionEpoch=6u,stage=CaptureStreamStage.FrozenLossless))
            backend.pending!!.losslessPaint!!.complete(Unit)
            assertFalse(withTimeout(5000){adopting.await()})
            assertEquals(frame,backend.positive?.frame)
            assertFalse(view.state.value.lossless)
        }finally{view.close();scope.cancel()}
    }
}
