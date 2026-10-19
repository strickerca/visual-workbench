package com.visualworkbench.desktop

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.material.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.ToneControlPreview
import kotlinx.coroutines.*
import org.jetbrains.skia.Image as SkiaImage
import java.awt.image.BufferedImage
import java.io.OutputStream
import java.util.concurrent.atomic.AtomicReference
import javax.imageio.ImageIO
import javax.imageio.stream.MemoryCacheImageOutputStream

/** An unpublished allocation can be closed immediately. After publication the
 * Compose/Skia renderer may retain it, so its wrapper follows normal GC lifetime. */
internal class TonePreviewBitmap(val display: ImageBitmap, private val native: SkiaImage) {
    fun abandon() { native.close() }
}

@Composable
internal fun TonePreviewTile(preview: ToneControlPreview) {
    val displayed by produceState<Triple<ToneControlPreview,TonePreviewBitmap?,Boolean>?>(null,preview) {
        value = null
        val handoff = AtomicReference<TonePreviewBitmap?>(null)
        try {
            val bitmap = withContext(Dispatchers.Default) { tonePreviewBitmap(preview).also(handoff::set) }
            currentCoroutineContext().ensureActive()
            value = Triple(preview,bitmap,false)
            handoff.set(null)
        } catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) { value = Triple(preview,null,true) }
        finally { handoff.getAndSet(null)?.abandon() }
    }
    val admitted = displayed?.takeIf { it.first === preview }
    if (admitted?.third == true) { Text("This preview tile could not be displayed."); return }
    val image = admitted?.second
    if (image == null) { Text("Preparing the bounded display tile…"); return }
    Box(Modifier.fillMaxWidth().heightIn(max=300.dp).aspectRatio(preview.width.toFloat()/preview.height.toFloat())) {
        Canvas(Modifier.fillMaxSize()) {
            val step = 16.dp.toPx()
            for (y in 0..(size.height/step).toInt()) for (x in 0..(size.width/step).toInt())
                drawRect(if ((x+y)%2 == 0) Color(0xffe0e0e0) else Color(0xffaaaaaa),Offset(x*step,y*step),Size(step,step))
        }
        Image(image.display,"Tone preview in sRGB; transparency shown on a checkerboard",Modifier.fillMaxSize(),contentScale=ContentScale.Fit)
    }
}

private suspend fun tonePreviewBitmap(value: ToneControlPreview): TonePreviewBitmap {
    require(value.width in 1u..512u && value.height in 1u..512u &&
        value.rgba.size.toULong() == value.width.toULong()*value.height.toULong()*4uL)
    val context = currentCoroutineContext()
    val width = value.width.toInt(); val height = value.height.toInt()
    context.ensureActive()
    val image = BufferedImage(width,height,BufferedImage.TYPE_INT_ARGB)
    try {
        val row = IntArray(width)
        for (y in 0 until height) {
            context.ensureActive()
            for (x in 0 until width) row[x] = toneArgb(value.rgba,(y*width+x)*4)
            image.setRGB(0,y,width,1,row,0,width)
        }
        val encoded = TonePngBuffer()
        MemoryCacheImageOutputStream(encoded).use { target -> check(ImageIO.write(image,"png",target)) }
        context.ensureActive()
        val native = SkiaImage.makeFromEncoded(encoded.bytes())
        try { context.ensureActive(); return TonePreviewBitmap(native.toComposeImageBitmap(),native) }
        catch (failure: Throwable) { native.close(); throw failure }
    } finally { image.flush() }
}

private fun toneArgb(bytes: ByteArray, offset: Int): Int =
    ((bytes[offset+3].toInt() and 255) shl 24) or ((bytes[offset].toInt() and 255) shl 16) or
        ((bytes[offset+1].toInt() and 255) shl 8) or (bytes[offset+2].toInt() and 255)

/** Fixed in-memory encoder cap; never changes ImageIO's global cache policy. */
private class TonePngBuffer : OutputStream() {
    private val buffer = ByteArray(2*1024*1024)
    private var size = 0
    override fun write(value: Int) { check(size < buffer.size); buffer[size++] = value.toByte() }
    override fun write(bytes: ByteArray, offset: Int, length: Int) {
        require(offset >= 0 && length >= 0 && offset <= bytes.size-length && length <= buffer.size-size)
        bytes.copyInto(buffer,size,offset,offset+length); size += length
    }
    fun bytes(): ByteArray = buffer.copyOf(size)
}
