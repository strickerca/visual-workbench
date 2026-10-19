package com.visualworkbench.desktop.stream

import com.visualworkbench.shared.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import java.awt.BufferCapabilities
import java.awt.Graphics
import java.awt.ImageCapabilities
import java.awt.image.BufferedImage
import java.awt.image.BufferStrategy

class DesktopStreamPixelsTest {
    private val frame=CaptureStreamFrame("capture",7u,2u,2u,2u,14,29,1.25,3u,900L,1000L,"windows")
    private class Charge:SessionResourceCharge {var closes=0;override fun close(){closes++}}
    private class Resources:SessionResources {
        val charge=Charge();var requested:ULong?=null
        override suspend fun reserveForeign(bytes:ULong):SessionResourceCharge {requested=bytes;return charge}
    }
    private class Pixels(override val frame:CaptureStreamFrame,private val reply:ByteArray):CaptureStreamPixels {
        val rows=mutableListOf<Pair<UInt,UInt>>()
        override suspend fun rgbaRows(firstRow:UInt,rowCount:UInt):ByteArray {
            rows+=firstRow to rowCount
            return reply
        }
        override suspend fun close()=Unit
    }

    @Test fun exactRowsBecomeChargedPlatformPixelsAndKeepFullSourceDimensions()=runBlocking {
        val resources=Resources()
        val pixels=Pixels(frame,byteArrayOf(1,2,3,-1,4,5,6,-1,7,8,9,-1,10,11,12,-1))
        val raster=readStreamRaster(pixels,resources){error("Unexpected failed close")}
        assertEquals(listOf(0u to 2u),pixels.rows)
        assertEquals(streamRasterCharge(streamRasterSize(frame),16L),resources.requested)
        assertTrue(streamRasterCharge(streamRasterSize(frame),64L)>checkNotNull(resources.requested))
        assertEquals(0xff010203.toInt(),raster.image?.getRGB(0,0))
        assertEquals(0xff0a0b0c.toInt(),raster.image?.getRGB(1,1))
        assertEquals(2,raster.image?.width)
        raster.close();raster.close()
        assertEquals(1,resources.charge.closes)
    }

    @Test fun hidpiAdmissionCountsPhysicalPixelsAndRetainedDestinationCopies() {
        assertEquals(4L*100L*50L*4L,physicalCanvasBytes(100,50,2.0,2.0))
        assertEquals(4L*151L*76L,physicalCanvasBytes(100,50,1.501,1.501))
        val size=streamRasterSize(frame)
        val physical=physicalCanvasBytes(100,50,2.0,2.0)
        assertEquals((size.bytes.toLong()*3L+physical*STREAM_PLATFORM_SURFACE_COPIES+
            STREAM_ROW_REPLY_MAX+16L*1024L).toULong(),streamRasterCharge(size,physical))
        assertThrows(IllegalArgumentException::class.java){physicalCanvasBytes(8192,8192,3.0,3.0)}
    }

    @Test fun bufferSubmissionReceiptsOnlyAfterVisibleShowAndTreatsShowFailureAsUnknown()=runBlocking {
        val events=mutableListOf<String>()
        fun strategy(fail:Boolean)=object:BufferStrategy() {
            override fun getCapabilities()=BufferCapabilities(ImageCapabilities(false),ImageCapabilities(false),null)
            override fun getDrawGraphics():Graphics=BufferedImage(2,2,BufferedImage.TYPE_INT_ARGB).createGraphics()
            override fun contentsLost()=false
            override fun contentsRestored()=false
            override fun show(){events+="show";if(fail)error("platform show failed")}
        }
        val shown=StreamDrawPermit()
        assertTrue(shown.paint{submitStreamBuffer(strategy(false),{events+="draw"},{events+="sync"})})
        assertEquals(listOf("draw","show","sync"),events)
        assertEquals(true,shown.awaitDecision())
        events.clear()
        val unknown=StreamDrawPermit()
        assertThrows(IllegalStateException::class.java){
            unknown.paint{submitStreamBuffer(strategy(true),{events+="draw"},{events+="sync"})}
        }
        assertEquals(listOf("draw","show"),events)
        assertNull(unknown.awaitDecision())
    }

    @Test fun boundsAndMalformedRowFailBeforeOrWithinChargedOwnership()=runBlocking {
        val resources=Resources()
        assertThrows(IllegalArgumentException::class.java){streamRasterSize(frame.copy(width=8193u))}
        assertThrows(IllegalArgumentException::class.java){streamRasterSize(frame.copy(width=4096u,height=4096u))}
        assertNull(resources.requested)
        val pixels=Pixels(frame,byteArrayOf(1,2,3))
        assertThrows(IllegalArgumentException::class.java){runBlocking {readStreamRaster(pixels,resources){error("Unexpected quarantine")}}}
        assertEquals(1,resources.charge.closes)
    }

    @Test fun drawPermitSettlesOnlyAfterPaintOrCertainPrepaintRefusal()=runBlocking {
        val shown=StreamDrawPermit();var paints=0
        assertTrue(shown.paint{paints++});assertEquals(true,shown.awaitDecision())
        assertFalse(shown.refuse());assertTrue(shown.paint{paints++});assertEquals(2,paints)
        val hidden=StreamDrawPermit();assertTrue(hidden.refuse())
        assertFalse(hidden.paint{paints++});assertEquals(false,hidden.awaitDecision())
        val unknown=StreamDrawPermit()
        assertThrows(IllegalStateException::class.java){unknown.paint{error("paint failed")}}
        assertNull(unknown.awaitDecision());assertFalse(unknown.refuse())
    }

    @Test fun losslessAdmissionRequiresExactDraftFrameAndEpoch() {
        val info=CaptureDraftInfo("draft","document",frame,"b".repeat(64),5u)
        val frozen=CaptureFreezeReceipt("draft",frame.frameId,frame.captureSessionId,frame.sourceGeneration)
        val state=CaptureAnnotationState(frozen=frozen,draft=info,lossless=true)
        val signal=CaptureStreamSignal(1u,5u,frame.sourceGeneration,frame.frameId,CaptureStreamStage.FrozenLossless,
            frame.width,frame.height,0u,null)
        fun bg(value:CaptureStreamFrame=frame,asset:String?="c".repeat(64),isLossless:Boolean=true)=object:CaptureFrozenBackground {
            override val frame=value;override val draftId="draft";override val lossless=isLossless;override val originalAssetId=asset
            override suspend fun rgbaRows(firstRow:UInt,rowCount:UInt)=byteArrayOf()
            override suspend fun close()=Unit
        }
        assertTrue(losslessBackgroundMatches(state,bg(),5u,signal))
        assertFalse(losslessBackgroundMatches(state,bg(value=frame.copy(frameId=8u)),5u,signal))
        assertFalse(losslessBackgroundMatches(state,bg(asset=null),5u,signal))
        assertFalse(losslessBackgroundMatches(state,bg(isLossless=false),5u,signal))
        assertFalse(losslessBackgroundMatches(state,bg(),6u,signal))
        assertFalse(losslessBackgroundMatches(state.copy(lossless=false),bg(),5u,signal))
    }
}
