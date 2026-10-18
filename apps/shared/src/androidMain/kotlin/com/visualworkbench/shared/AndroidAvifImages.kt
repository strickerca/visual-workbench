package com.visualworkbench.shared

import android.graphics.Bitmap
import android.graphics.ColorSpace
import android.graphics.ImageDecoder
import com.visualworkbench.bindings.core.AndroidAvifDecoder
import com.visualworkbench.bindings.core.AvifImageException
import com.visualworkbench.bindings.core.AvifImagePixels
import com.visualworkbench.bindings.core.AvifImageRequest
import com.visualworkbench.bindings.core.installAndroidAvifDecoder
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean

/** Installed exactly once after native load. No Context, project, Activity or
 * capture permission is retained. Called synchronously on an owned core worker.
 * ImageDecoder is not interruptible: its native capacity remains occupied until
 * it returns, and caller cancellation is checked before exposing the result. */
internal object AndroidAvifImages:AndroidAvifDecoder {
    private val active=AtomicBoolean(false)
    private var installed=false
    @Synchronized fun install(){if(!installed){installAndroidAvifDecoder(this);installed=true}}
    override fun decode(request:AvifImageRequest):AvifImagePixels {
        val info=request.info
        admitAndroidAvif(request)
        val width=info.width.toLong();val height=info.height.toLong()
        if(!ImageDecoder.isMimeTypeSupported("image/avif"))throw AvifImageException.Unavailable()
        if(!active.compareAndSet(false,true))throw AvifImageException.Busy()
        var bitmap:Bitmap?=null
        try {
            val source=ImageDecoder.createSource(ByteBuffer.wrap(request.encoded).asReadOnlyBuffer())
            bitmap=ImageDecoder.decodeBitmap(source){decoder,header,_->
                if(header.isAnimated||header.mimeType != "image/avif")throw AvifImageException.Unsupported()
                if(header.size.width.toLong()!=width||header.size.height.toLong()!=height)throw AvifImageException.Orientation()
                if(header.colorSpace?.isSrgb!=true)throw AvifImageException.Color()
                decoder.allocator=ImageDecoder.ALLOCATOR_SOFTWARE
                decoder.memorySizePolicy=ImageDecoder.MEMORY_POLICY_DEFAULT
                decoder.isUnpremultipliedRequired=true
                decoder.setTargetColorSpace(ColorSpace.get(ColorSpace.Named.SRGB))
                decoder.setOnPartialImageListener { false }
            }
            val image=checkNotNull(bitmap)
            if(image.width.toLong()!=width||image.height.toLong()!=height||image.config!=Bitmap.Config.ARGB_8888||image.colorSpace?.isSrgb!=true)throw AvifImageException.Decode()
            // Account actual bitmap storage before allocating interop copies.
            // The native estimate is conservative, not a measured RSS bound.
            val retained=image.allocationByteCount.toLong()
            if(retained<=0)throw AvifImageException.Memory()
            val replyBytes=width.toULong()*height.toULong()*4uL
            val actualPeak=retained.toULong()+replyBytes*4uL+width.toULong()*4uL+
                request.encoded.size.toULong()*4uL+32uL*1024uL*1024uL
            if(actualPeak>request.memoryBudgetBytes || actualPeak>info.estimatedPeakBytes)throw AvifImageException.Memory()
            val rgba=ByteArray(Math.toIntExact(width*height*4));val row=IntArray(width.toInt())
            for(y in 0 until height.toInt()){
                image.getPixels(row,0,width.toInt(),0,y,width.toInt(),1)
                for(x in row.indices){val pixel=row[x];if(pixel ushr 24!=255)throw AvifImageException.Unsupported();val at=(y*width.toInt()+x)*4
                    rgba[at]=(pixel ushr 16).toByte();rgba[at+1]=(pixel ushr 8).toByte();rgba[at+2]=pixel.toByte();rgba[at+3]=255.toByte()
                }
            }
            return AvifImagePixels(info.width,info.height,rgba)
        }catch(error:AvifImageException){throw error}
        catch(_:ImageDecoder.DecodeException){throw AvifImageException.Decode()}
        catch(_:OutOfMemoryError){throw AvifImageException.Memory()}
        catch(_:Exception){throw AvifImageException.Decode()}
        finally{try{bitmap?.recycle()}finally{active.set(false)}}
    }
}

/** No OS calls. The native original/OBU admission remains mandatory; repeat
 * callback resource checks here instead of trusting a supplied estimate. */
internal fun admitAndroidAvif(request:AvifImageRequest) {
    val info=request.info
    if(info.sourceAssetId.length!=64 || info.sourceAssetId.any { it !in "0123456789abcdef" })throw AvifImageException.Invalid()
    if(info.orientation!=1.toUByte())throw AvifImageException.Orientation()
    if(info.bitDepth!=8.toUByte())throw AvifImageException.Depth()
    // ImageDecoder cannot expose arbitrary raw ICC sample preservation. The
    // native adapter admits exact sRGB nclx for this profile-free path.
    if(info.iccProfile.isNotEmpty())throw AvifImageException.Color()
    val color=info.nclx?:throw AvifImageException.Color()
    if(color.primaries!=1.toUShort()||color.transfer!=13.toUShort()||color.matrix !in setOf(1.toUShort(),6.toUShort()))throw AvifImageException.Color()
    val width=info.width.toULong();val height=info.height.toULong()
    if(width !in 1uL..32768uL||height !in 1uL..32768uL||width*height>50_000_000uL||request.encoded.isEmpty()||request.encoded.size>64*1024*1024||request.memoryBudgetBytes>512uL*1024uL*1024uL)throw AvifImageException.Memory()
    val coded=((width+127uL)/128uL*128uL)*((height+127uL)/128uL*128uL)
    val peak=coded*64uL+request.encoded.size.toULong()*4uL+32uL*1024uL*1024uL
    if(peak!=info.estimatedPeakBytes||peak>request.memoryBudgetBytes)throw AvifImageException.Memory()
}
