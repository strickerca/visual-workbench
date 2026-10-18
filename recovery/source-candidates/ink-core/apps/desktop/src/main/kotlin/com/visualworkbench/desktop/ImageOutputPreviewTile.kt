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
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.AiRegion
import com.visualworkbench.shared.ImageOutputPreview
import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicReference

/** Reuses only the bounded RGBA display converter. No AI service, credentials,
 * provider, request or source image is acquired by an output preview. */
@Composable
internal fun ImageOutputPreviewTile(preview: ImageOutputPreview) {
    val displayed by produceState<Triple<ImageOutputPreview,AiBitmap?,Boolean>?>(null,preview) {
        value=null
        try{value=Triple(preview,outputBitmap(preview),false)}catch(cancel:CancellationException){throw cancel}
        catch(_:Exception){value=Triple(preview,null,true)}
    }
    val admitted=displayed?.takeIf{it.first===preview}
    if(admitted?.third==true){Text("This committed tile could not be displayed. The saved output and original remain intact.");return}
    val image=admitted?.second
    if(image==null){Text("Preparing the bounded display tile…");return}
    Box(Modifier.fillMaxWidth().heightIn(max=300.dp).aspectRatio(preview.width.toFloat()/preview.height.toFloat())) {
        Canvas(Modifier.fillMaxSize()) {
            val step=16.dp.toPx()
            for(y in 0..(size.height/step).toInt())for(x in 0..(size.width/step).toInt())
                drawRect(if((x+y)%2==0)Color(0xffe0e0e0)else Color(0xffaaaaaa),Offset(x*step,y*step),Size(step,step))
        }
        Image(image.display,"Committed output preview in sRGB; transparency shown on a checkerboard",
            Modifier.fillMaxSize(),contentScale=ContentScale.Fit)
    }
}

private suspend fun outputBitmap(value:ImageOutputPreview):AiBitmap {
    val held=AtomicReference<AiBitmap?>(null)
    try {
        val result=withContext(Dispatchers.Default){
            aiBitmap(AiRegion(value.x,value.y,value.width,value.height),value.outputWidth,value.outputHeight,value.rgba).also(held::set)
        }
        currentCoroutineContext().ensureActive();held.set(null);return result
    }finally{held.getAndSet(null)?.abandon()}
}
