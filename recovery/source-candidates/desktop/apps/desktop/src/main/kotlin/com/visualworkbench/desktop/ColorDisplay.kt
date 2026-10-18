package com.visualworkbench.desktop

import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.jetbrains.skia.ColorAlphaType
import org.jetbrains.skia.ColorSpace
import org.jetbrains.skia.ColorType
import org.jetbrains.skia.Image
import org.jetbrains.skia.ImageInfo

/** Skia performs source→surface conversion from this explicit image tag. This
 * does not claim that Compose's window is an HDR or monitor-ICC surface. */
internal object ColorDisplay {
    suspend fun bitmap(value:DisplayColorPixels,request:DisplayColorRequest):OwnedColorImage {
        validateDisplayPixels(value,request)
        currentCoroutineContext().ensureActive()
        var owned:Image?=null
        try {
            withContext(Dispatchers.Default) {
                val space=if(value.space==DisplayColorSpace.DisplayP3)ColorSpace.displayP3 else ColorSpace.sRGB
                owned=Image.makeRaster(ImageInfo(value.region.width.toInt(),value.region.height.toInt(),ColorType.RGBA_8888,ColorAlphaType.UNPREMUL,space),value.rgba,value.region.width.toInt()*4)
            }
            currentCoroutineContext().ensureActive()
            return OwnedColorImage(checkNotNull(owned)).also{owned=null}
        } finally {owned?.close()}
    }

    /** background() documents an explicit sRGB display conversion. Preserve that
     * interpretation for the bounded legacy path; never retag original pixels. */
    suspend fun legacy(value: BackgroundImage): OwnedColorImage {
        val n = value.width.toULong() * value.height.toULong()
        require(value.width in 1u..50_000u && value.height in 1u..50_000u && n <= 50_000_000uL && n * 4uL == value.rgba.size.toULong())
        currentCoroutineContext().ensureActive()
        var owned: Image? = null
        try {
            withContext(Dispatchers.Default) {
                owned = Image.makeRaster(ImageInfo(value.width.toInt(), value.height.toInt(), ColorType.RGBA_8888,
                    ColorAlphaType.UNPREMUL, ColorSpace.sRGB), value.rgba, value.width.toInt() * 4)
            }
            currentCoroutineContext().ensureActive()
            return OwnedColorImage(checkNotNull(owned)).also { owned = null }
        } finally { owned?.close() }
    }
}

/** A cancelled or stale result releases its native image. Once adopted, Compose
 * and any published draw snapshots retain it until normal reachability releases
 * the image; it must never be explicitly closed underneath a draw. */
internal class OwnedColorImage(internal val skia: Image) {
    private var owned = true
    val image: ImageBitmap = skia.toComposeImageBitmap()
    fun take(): ImageBitmap { check(owned); owned = false; return image }
    fun abandon() { if (owned) { owned = false; skia.close() } }
}
