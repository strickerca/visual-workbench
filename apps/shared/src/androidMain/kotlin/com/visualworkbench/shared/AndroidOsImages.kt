package com.visualworkbench.shared

import android.graphics.Bitmap
import android.graphics.ColorSpace
import android.graphics.ImageDecoder
import com.visualworkbench.bindings.core.AndroidImageDecoder
import com.visualworkbench.bindings.core.OsImageException
import com.visualworkbench.bindings.core.OsImagePixels
import com.visualworkbench.bindings.core.OsImageRequest
import com.visualworkbench.bindings.core.installAndroidImageDecoder
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean

/** Installed exactly once after native load. No Context, project, Activity or
 * capture permission is retained. Called synchronously on an owned core worker.
 * ImageDecoder is not interruptible: its native capacity remains occupied until
 * it returns, and caller cancellation is checked before exposing the result. */
internal object AndroidOsImages:AndroidImageDecoder {
    private val active=AtomicBoolean(false)
    private var installed=false
    @Synchronized fun install(){if(!installed){installAndroidImageDecoder(this);installed=true}}
    override fun decode(request:OsImageRequest):OsImagePixels {
        val info=request.info
        if(info.orientation!=1.toUByte())throw OsImageException.Orientation()
        if(info.bitDepth!=8.toUByte())throw OsImageException.Depth()
        // ImageDecoder does not expose the original ICC bytes or guarantee
        // arbitrary-profile samples without a conversion. Refuse that path.
        if(info.iccProfile.isNotEmpty())throw OsImageException.Color()
        val width=info.width.toLong();val height=info.height.toLong()
        if(width !in 1..32768||height !in 1..32768||width*height>50_000_000||request.encoded.size>64*1024*1024||info.estimatedPeakBytes>request.memoryBudgetBytes||request.memoryBudgetBytes>512uL*1024uL*1024uL)throw OsImageException.Memory()
        if(!ImageDecoder.isMimeTypeSupported("image/heic"))throw OsImageException.Unavailable()
        if(!active.compareAndSet(false,true))throw OsImageException.Busy()
        var bitmap:Bitmap?=null
        try {
            val source=ImageDecoder.createSource(ByteBuffer.wrap(request.encoded).asReadOnlyBuffer())
            bitmap=ImageDecoder.decodeBitmap(source){decoder,header,_->
                if(header.isAnimated||header.mimeType !in setOf("image/heic","image/heif"))throw OsImageException.Unsupported()
                if(header.size.width.toLong()!=width||header.size.height.toLong()!=height)throw OsImageException.Orientation()
                if(header.colorSpace?.isSrgb!=true)throw OsImageException.Color()
                decoder.allocator=ImageDecoder.ALLOCATOR_SOFTWARE
                decoder.memorySizePolicy=ImageDecoder.MEMORY_POLICY_DEFAULT
                decoder.isUnpremultipliedRequired=true
                decoder.setTargetColorSpace(ColorSpace.get(ColorSpace.Named.SRGB))
                decoder.setOnPartialImageListener { false }
            }
            val image=checkNotNull(bitmap)
            if(image.width.toLong()!=width||image.height.toLong()!=height||image.config!=Bitmap.Config.ARGB_8888||image.colorSpace?.isSrgb!=true)throw OsImageException.Decode()
            val rgba=ByteArray(Math.toIntExact(width*height*4));val row=IntArray(width.toInt())
            for(y in 0 until height.toInt()){
                image.getPixels(row,0,width.toInt(),0,y,width.toInt(),1)
                for(x in row.indices){val pixel=row[x];if(pixel ushr 24!=255)throw OsImageException.Unsupported();val at=(y*width.toInt()+x)*4
                    rgba[at]=(pixel ushr 16).toByte();rgba[at+1]=(pixel ushr 8).toByte();rgba[at+2]=pixel.toByte();rgba[at+3]=255.toByte()
                }
            }
            return OsImagePixels(info.width,info.height,rgba)
        }catch(error:OsImageException){throw error}
        catch(_:ImageDecoder.DecodeException){throw OsImageException.Decode()}
        catch(_:OutOfMemoryError){throw OsImageException.Memory()}
        catch(_:Exception){throw OsImageException.Decode()}
        finally{try{bitmap?.recycle()}finally{active.set(false)}}
    }
}
