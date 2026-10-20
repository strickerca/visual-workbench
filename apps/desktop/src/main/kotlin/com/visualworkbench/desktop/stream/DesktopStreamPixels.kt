package com.visualworkbench.desktop.stream

import com.visualworkbench.shared.*
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.awt.image.BufferedImage
import java.awt.image.DataBufferInt

internal const val STREAM_ROW_REPLY_MAX = 256 * 1024
internal const val STREAM_FRAME_RGBA_MAX = 32 * 1024 * 1024
internal const val STREAM_SURFACE_RGBA_MAX = 64 * 1024 * 1024
// Admit the full supported native envelope once before BufferStrategy exists.
// It remains held even if resize/DPI changes the backing before the next draw.
internal const val STREAM_PLATFORM_SURFACE_COPIES = 3L
internal const val STREAM_PLATFORM_SURFACE_RESERVE = STREAM_SURFACE_RGBA_MAX.toLong()*STREAM_PLATFORM_SURFACE_COPIES

internal data class StreamRasterSize(val width:Int,val height:Int,val rowBytes:Int,val bytes:Int)

/** Source RGBA and Java image are held with a third source-sized workspace;
 * native destination buffers have their own view-lifetime reservation. */
internal fun streamRasterSize(frame:CaptureStreamFrame):StreamRasterSize {
    val width=frame.width.toLong();val height=frame.height.toLong()
    require(width in 1..8192 && height in 1..8192 && frame.dpiScale.isFinite() && frame.dpiScale>0.0)
    require(frame.captureSessionId.isNotEmpty()&&frame.captureSessionId.length<=128)
    val bytes=width*height*4L
    require(bytes in 4..STREAM_FRAME_RGBA_MAX.toLong())
    val rowBytes=width*4L
    require(rowBytes<=STREAM_ROW_REPLY_MAX)
    return StreamRasterSize(width.toInt(),height.toInt(),rowBytes.toInt(),bytes.toInt())
}

internal fun losslessBackgroundMatches(state:CaptureAnnotationState,background:CaptureFrozenBackground,
    connectionEpoch:ULong,current:CaptureStreamSignal):Boolean {
    val draft=state.draft?:return false
    return state.lossless&&state.frozen?.draftId==draft.draftId&&
        background.lossless&&background.originalAssetId!=null&&background.draftId==draft.draftId&&
        background.frame==draft.frame&&current.connectionEpoch==connectionEpoch
}
internal fun streamRasterCharge(size:StreamRasterSize):ULong =
    (size.bytes.toLong()*3L+STREAM_ROW_REPLY_MAX+16L*1024L).toULong()

internal class StreamSurfaceLease(private val charge:SessionResourceCharge) {
    val capacityBytes:Long=STREAM_SURFACE_RGBA_MAX.toLong()
    private var closed=false
    private var failure:Throwable?=null
    fun requireAdmitted(actualBytes:Long){check(!closed&&failure==null);require(actualBytes in 4..capacityBytes)}
    fun close(){
        failure?.let{throw it}
        if(closed)return
        try{charge.close();closed=true}catch(error:Throwable){failure=error;throw error}
    }
}
internal suspend fun reserveStreamSurface(resources:SessionResources):StreamSurfaceLease =
    StreamSurfaceLease(resources.reserveForeign(STREAM_PLATFORM_SURFACE_RESERVE.toULong()))

/** A failed charge release stays owned; a second call reports the same failure. */
internal class StreamRaster(private val charge:SessionResourceCharge,val admittedSurfaceBytes:Long) {
    var image:BufferedImage?=null;private set
    private var closed=false
    private var failure:Throwable?=null
    fun attach(value:BufferedImage){check(image==null&&!closed);image=value}
    fun close(){
        failure?.let{throw it}
        if(closed)return
        try{image?.flush();charge.close();closed=true;image=null}
        catch(error:Throwable){failure=error;throw error}
    }
}

/** The only paint/false-decision race. drawImage must finish before the permit
 * reports true; a known rejection can never become a later positive receipt. */
internal class StreamDrawPermit {
    private val decision=CompletableDeferred<Boolean?>()
    private var state=0 // 0 pending, 1 actually drawn, -1 rejected before paint, 2 unknown paint
    @Synchronized fun paint(draw:()->Unit):Boolean {
        if(state<0||state==2)return false
        try {
            draw()
            if(state==0){state=1;decision.complete(true)}
            return true
        } catch(error:Throwable){state=2;decision.complete(null);throw error}
    }
    @Synchronized fun refuse():Boolean {
        if(state!=0)return false
        state=-1;decision.complete(false);return true
    }
    suspend fun awaitDecision():Boolean? = decision.await()
    @Synchronized fun canDisplay():Boolean=state>=0&&state!=2
}

/** Reads exact opaque RGBA rows; no full-frame native reply is requested. */
internal suspend fun readStreamRaster(pixels:CaptureStreamPixels,resources:SessionResources,
    surface:StreamSurfaceLease,platformSurfaceBytes:Long,quarantine:(StreamRaster)->Unit):StreamRaster {
    val size=streamRasterSize(pixels.frame)
    surface.requireAdmitted(platformSurfaceBytes)
    val held=StreamRaster(resources.reserveForeign(streamRasterCharge(size)),surface.capacityBytes)
    try {
        val rgba=ByteArray(size.bytes)
        var row=0
        val maxRows=STREAM_ROW_REPLY_MAX/size.rowBytes
        while(row<size.height){
            currentCoroutineContext().ensureActive()
            val count=minOf(maxRows,size.height-row)
            val chunk=pixels.rgbaRows(row.toUInt(),count.toUInt())
            require(chunk.size==count*size.rowBytes)
            chunk.copyInto(rgba,row*size.rowBytes)
            row+=count
        }
        currentCoroutineContext().ensureActive()
        require((3 until rgba.size step 4).all{rgba[it]==(-1).toByte()})
        val image=BufferedImage(size.width,size.height,BufferedImage.TYPE_INT_ARGB)
        held.attach(image)
        val argb=(image.raster.dataBuffer as DataBufferInt).data
        for(index in argb.indices){
            if(index%65_536==0)currentCoroutineContext().ensureActive()
            val offset=index*4
            argb[index]=((rgba[offset+3].toInt() and 255) shl 24) or
                ((rgba[offset].toInt() and 255) shl 16) or
                ((rgba[offset+1].toInt() and 255) shl 8) or
                (rgba[offset+2].toInt() and 255)
        }
        return held
    } catch(error:Throwable) {
        withContext(NonCancellable){
            try{held.close()}catch(cleanup:Throwable){quarantine(held);error.addSuppressed(cleanup)}
        }
        throw error
    }
}
