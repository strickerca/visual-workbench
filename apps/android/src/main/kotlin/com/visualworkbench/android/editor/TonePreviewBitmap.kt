package com.visualworkbench.android.editor

import android.graphics.Bitmap
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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.ToneControlPreview
import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicReference

/** Only unpublished bitmaps may be recycled. A published image can still be
 * referenced by Android's render thread after the panel replaces its state. */
internal class TonePreviewBitmap(private val native: Bitmap) {
    val display: ImageBitmap = native.asImageBitmap()
    fun abandon() { native.recycle() }
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
    val image = Bitmap.createBitmap(width,height,Bitmap.Config.ARGB_8888)
    try {
        val row = IntArray(width)
        for (y in 0 until height) {
            context.ensureActive()
            for (x in 0 until width) {
                val offset = (y*width+x)*4
                row[x] = ((value.rgba[offset+3].toInt() and 255) shl 24) or
                    ((value.rgba[offset].toInt() and 255) shl 16) or
                    ((value.rgba[offset+1].toInt() and 255) shl 8) or (value.rgba[offset+2].toInt() and 255)
            }
            image.setPixels(row,0,width,0,y,width,1)
        }
        context.ensureActive()
        return TonePreviewBitmap(image)
    } catch (failure: Throwable) { image.recycle(); throw failure }
}
