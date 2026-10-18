package com.visualworkbench.android.editor

import android.app.Activity
import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.graphics.ColorSpace
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import java.nio.ByteBuffer

/** The tag travels with the pixels. Window support affects gamut presentation,
 * never the interpretation of stored values or the immutable original. */
internal object ColorDisplay {
    fun enable(windowOwner:Activity) { windowOwner.window.colorMode=ActivityInfo.COLOR_MODE_WIDE_COLOR_GAMUT }
    fun space(windowOwner:Activity):DisplayColorSpace =
        if(windowOwner.window.isWideColorGamut&&windowOwner.resources.configuration.isScreenWideColorGamut)
            DisplayColorSpace.DisplayP3 else DisplayColorSpace.Srgb

    suspend fun bitmap(value:DisplayColorPixels, request:DisplayColorRequest):Bitmap {
        validateDisplayPixels(value,request)
        currentCoroutineContext().ensureActive()
        var owned:Bitmap?=null
        try {
            withContext(Dispatchers.Default) {
                val w=value.region.width.toInt();val h=value.region.height.toInt()
                val color=ColorSpace.get(if(value.space==DisplayColorSpace.DisplayP3)ColorSpace.Named.DISPLAY_P3 else ColorSpace.Named.SRGB)
                val out=Bitmap.createBitmap(null,w,h,Bitmap.Config.ARGB_8888,true,color)
                owned=out
                check(out.rowBytes==w*4&&out.byteCount==value.rgba.size)
                // copyPixelsFromBuffer copies raw components without the sRGB
                // conversion performed by setPixels(int[]). Android RGBA_8888
                // stores RGBA bytes. Write components explicitly, independent of
                // host endianness, and premultiply without an sRGB conversion.
                val upload=ByteBuffer.allocate(value.rgba.size)
                for(y in 0 until h){ensureActive();for(x in 0 until w){
                    val i=(y*w+x)*4;val a=value.rgba[i+3].toInt() and 255
                    fun p(c:Int)=((value.rgba[i+c].toInt() and 255)*a+127)/255
                    upload.put(p(0).toByte());upload.put(p(1).toByte());upload.put(p(2).toByte());upload.put(a.toByte())
                }}
                upload.rewind();out.copyPixelsFromBuffer(upload);out.setPremultiplied(true)
            }
            currentCoroutineContext().ensureActive()
            return checkNotNull(owned).also{owned=null}
        } finally {owned?.recycle()}
    }

    /** The legacy core explicitly converts its display derivative to sRGB. */
    suspend fun legacy(value: BackgroundImage): Bitmap {
        val n = value.width.toULong() * value.height.toULong()
        require(value.width in 1u..50_000u && value.height in 1u..50_000u && n <= 50_000_000uL && n * 4uL == value.rgba.size.toULong())
        currentCoroutineContext().ensureActive()
        var owned: Bitmap? = null
        try {
            withContext(Dispatchers.Default) {
                val w = value.width.toInt(); val h = value.height.toInt()
                val out = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888, true, ColorSpace.get(ColorSpace.Named.SRGB))
                owned = out
                val row = IntArray(w)
                for (y in 0 until h) {
                    ensureActive()
                    for (x in row.indices) {
                        val i = (y * w + x) * 4
                        row[x] = ((value.rgba[i + 3].toInt() and 255) shl 24) or
                            ((value.rgba[i].toInt() and 255) shl 16) or ((value.rgba[i + 1].toInt() and 255) shl 8) or
                            (value.rgba[i + 2].toInt() and 255)
                    }
                    out.setPixels(row, 0, w, 0, y, w, 1)
                }
            }
            currentCoroutineContext().ensureActive()
            return checkNotNull(owned).also { owned = null }
        } finally { owned?.recycle() }
    }
}
