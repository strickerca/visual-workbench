package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.layout.ContentScale
import kotlinx.coroutines.*
import org.jetbrains.skia.Image as SkiaImage
import java.awt.image.BufferedImage
import java.io.OutputStream
import java.util.concurrent.atomic.AtomicReference
import javax.imageio.ImageIO
import javax.imageio.stream.MemoryCacheImageOutputStream

/** One private complete frame. Only explicit Apply/Remove edits the document.
 * The caller owns activation and composes the source-space editing viewport. */
@Composable
internal fun ImageFramesPanel(controls: ImageFramesInteraction, activate: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val state by controls.state.collectAsState()
    val opening = controls.capture()
    OutlinedButton(onClick={if (opening != null && controls.capture() == opening) activate(true)},
        enabled=opening != null,modifier=modifier.heightIn(min=48.dp)) {
        Text(if (state.busy) "Output frame · saving…" else if (state.retry != null) "Output frame · retry retained" else "Output frame")
    }
    if (!state.active) return
    val token = controls.capture()
    val snapshot = state.snapshot
    val form = state.form
    val locked = snapshot?.frame?.locked == true
    val editable = state.ready && token != null && !locked
    var previewX by remember(token) { mutableStateOf("0") }
    var previewY by remember(token) { mutableStateOf("0") }
    var draft by remember(token) { mutableStateOf(true) }
    var marked by remember(token) { mutableStateOf(true) }
    var assume by remember(token) { mutableStateOf(false) }
    fun current(): Boolean = token != null && controls.capture() == token
    fun change(update: (ImageFramesForm) -> ImageFramesForm) {
        if (token == null || !current()) return
        val latest = controls.state.value.form ?: return
        controls.edit(token,update(latest))
    }
    fun local(change: () -> Unit) {
        if (current()) { change(); token?.let { controls.invalidatePreview(it) } }
    }
    val x = frameCoordinate(previewX); val y = frameCoordinate(previewY)
    AlertDialog(onDismissRequest={if (current()) activate(false)},title={Text("Output frame")},text={
        Column(Modifier.fillMaxWidth().heightIn(min=240.dp,max=600.dp).verticalScroll(rememberScrollState()),
            verticalArrangement=Arrangement.spacedBy(8.dp)) {
            Text("Crop, quarter-turn rotation and output size are editable parameters. Every original source pixel remains available in the editor and Undo restores the prior frame.",
                style=MaterialTheme.typography.body2)
            snapshot?.let {
                Text("Original source: ${it.sourceWidth} × ${it.sourceHeight} · ${it.sourceBitDepth}-bit. Saved output: ${it.outputWidth} × ${it.outputHeight}.",
                    style=MaterialTheme.typography.caption)
            }
            if (state.loading || state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (state.stale) Text("The visible revision changed. Private values are retained; discard them and reload before applying.")
            if (locked) Text("This saved frame is locked. Unlock its object and layer in Documents and layers before editing it.")
            state.issue?.let { Text(it,color=MaterialTheme.colors.secondary) }
            state.lastReceipt?.let { Text("Saved at revision ${it.revision.hostSeq}.",style=MaterialTheme.typography.caption) }
            if (state.retry != null) {
                Text("One exact edit is retained. Retry resolves whether it was saved; discarding the handle leaves saved history intact.")
                Row {
                    TextButton(onClick={token?.let { controls.retry(it) }},enabled=!state.busy && token != null) { Text("Retry exact edit") }
                    TextButton(onClick={token?.let { controls.discardRetry(it) }},enabled=!state.busy && token != null) { Text("Discard retry") }
                }
            }
            if (state.busy) TextButton(onClick={token?.let { controls.cancel(it) }},enabled=token != null) { Text("Cancel and settle") }
            TextButton(onClick={token?.let { controls.discardFormAndReload(it) }},
                enabled=!state.busy && state.retry == null && token != null) {
                Text(if (state.dirty) "Discard private values and reload" else "Reload saved frame")
            }
            if (form != null) {
                FrameToggle("Enable output frame",form.enabled,editable) { value -> change { it.copy(enabled=value) } }
                Text("Crop uses original source coordinates. Enter whole numbers; width and height must be positive and fit within the original.",
                    style=MaterialTheme.typography.caption)
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    FrameField("Crop left",form.cropX,editable,Modifier.weight(1f)) { value -> change { it.copy(cropX=value) } }
                    FrameField("Crop top",form.cropY,editable,Modifier.weight(1f)) { value -> change { it.copy(cropY=value) } }
                }
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    FrameField("Crop width",form.cropWidth,editable,Modifier.weight(1f)) { value -> change { it.copy(cropWidth=value) } }
                    FrameField("Crop height",form.cropHeight,editable,Modifier.weight(1f)) { value -> change { it.copy(cropHeight=value) } }
                }
                Text("Clockwise rotation",style=MaterialTheme.typography.subtitle2)
                Row(horizontalArrangement=Arrangement.spacedBy(4.dp)) {
                    listOf(0,90,180,270).forEach { degrees ->
                        TextButton(onClick={change { it.copy(clockwiseDegrees=degrees.toUShort()) }},enabled=editable) {
                            Text((if (form.clockwiseDegrees.toInt() == degrees) "✓ " else "") + "$degrees°")
                        }
                    }
                }
                Text("Choose both output dimensions explicitly. Aspect ratio is free; no values change automatically when rotating. Maximum 32768 per side and 50 million output pixels.",
                    style=MaterialTheme.typography.caption)
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    FrameField("Output width",form.outputWidth,editable,Modifier.weight(1f)) { value -> change { it.copy(outputWidth=value) } }
                    FrameField("Output height",form.outputHeight,editable,Modifier.weight(1f)) { value -> change { it.copy(outputHeight=value) } }
                }
                TextButton(onClick={if (snapshot != null) change {
                    it.copy(cropX="0",cropY="0",cropWidth=snapshot.sourceWidth.toString(),cropHeight=snapshot.sourceHeight.toString(),
                        clockwiseDegrees=0u,outputWidth=snapshot.sourceWidth.toString(),outputHeight=snapshot.sourceHeight.toString())
                }},enabled=editable) { Text("Use full original with no rotation") }
                Button(onClick={token?.let { controls.apply(it) }},enabled=editable,
                    modifier=Modifier.fillMaxWidth().heightIn(min=48.dp)) { Text("Apply complete frame") }
                TextButton(onClick={token?.let { controls.remove(it) }},
                    enabled=editable && !state.dirty && snapshot?.frame != null) { Text("Remove saved frame") }
            }
            if (state.dirty) Text("These private values are unsaved. Apply or discard them before closing.",style=MaterialTheme.typography.caption)
            Divider()
            Text("Output preview",style=MaterialTheme.typography.subtitle1)
            Text("One exact region of up to 512 × 512 output pixels, displayed as 8-bit sRGB. Left and top use the output grid after crop, rotation and resize. Preview creates no saved edit.",
                style=MaterialTheme.typography.caption)
            Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                FrameField("Output preview left",previewX,state.ready && token != null,Modifier.weight(1f)) { value -> local { previewX=value } }
                FrameField("Output preview top",previewY,state.ready && token != null,Modifier.weight(1f)) { value -> local { previewY=value } }
            }
            FrameToggle("Preview private frame",draft,state.ready && token != null) { value -> local { draft=value } }
            FrameToggle("Include saved annotations",marked,state.ready && token != null) { value -> local { marked=value } }
            FrameToggle("Treat untagged source colors as sRGB for this preview",assume,state.ready && token != null) { value -> local { assume=value } }
            OutlinedButton(onClick={if (token != null && x != null && y != null) controls.preview(token,x,y,assume,draft,marked)},
                enabled=state.ready && token != null && x != null && y != null && (!draft || !locked),
                modifier=Modifier.heightIn(min=48.dp)) { Text(if (draft) "Preview private frame" else "Preview saved frame") }
            if (state.previewLoading) LinearProgressIndicator(Modifier.fillMaxWidth())
            state.preview?.let { preview ->
                Text((if (state.previewDraft) "Private frame" else "Saved frame") + " · output ${preview.outputWidth} × ${preview.outputHeight}" +
                    " · revision ${preview.binding.hostSeq} · region [${preview.x}, ${preview.y}, ${preview.width}, ${preview.height}]" +
                    (if (state.previewMarked) " · saved annotations included" else " · without annotations"),style=MaterialTheme.typography.caption)
                FramePreviewTile(preview)
            }
        }
    },confirmButton={TextButton(onClick={if (current()) activate(false)},enabled=state.mayLeave && token != null) { Text("Close") }})
}

@Composable
private fun FrameField(label: String, value: String, enabled: Boolean, modifier: Modifier = Modifier, change: (String) -> Unit) {
    OutlinedTextField(value,{if (it.length <= 32 && it.all { digit -> digit in '0'..'9' }) change(it)},
        label={Text(label)},enabled=enabled,singleLine=true,modifier=modifier)
}
@Composable
private fun FrameToggle(label: String, value: Boolean, enabled: Boolean, change: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min=48.dp),verticalAlignment=Alignment.CenterVertically) {
        Checkbox(value,change,enabled=enabled); Text(label,Modifier.weight(1f),style=MaterialTheme.typography.body2)
    }
}
private fun frameCoordinate(text: String): UInt? =
    text.takeIf { it.isNotEmpty() && it.all { value -> value in '0'..'9' } }?.toUIntOrNull()

/** An unpublished allocation can be closed immediately. After publication the
 * Compose/Skia renderer may retain it, so its wrapper follows normal GC lifetime. */
private class FramePreviewBitmap(val display: ImageBitmap, private val native: SkiaImage) {
    fun abandon() { native.close() }
}

@Composable
private fun FramePreviewTile(preview: ImageFramePreview) {
    val displayed by produceState<Triple<ImageFramePreview,FramePreviewBitmap?,Boolean>?>(null,preview) {
        value = null
        val handoff = AtomicReference<FramePreviewBitmap?>(null)
        try {
            val bitmap = withContext(Dispatchers.Default) { framePreviewBitmap(preview).also(handoff::set) }
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
        Image(image.display,"Output frame preview in sRGB; transparency shown on a checkerboard",Modifier.fillMaxSize(),contentScale=ContentScale.Fit)
    }
}

private suspend fun framePreviewBitmap(value: ImageFramePreview): FramePreviewBitmap {
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
            for (x in 0 until width) row[x] = frameArgb(value.rgba,(y*width+x)*4)
            image.setRGB(0,y,width,1,row,0,width)
        }
        val encoded = FramePngBuffer()
        MemoryCacheImageOutputStream(encoded).use { target -> check(ImageIO.write(image,"png",target)) }
        context.ensureActive()
        val native = SkiaImage.makeFromEncoded(encoded.bytes())
        try { context.ensureActive(); return FramePreviewBitmap(native.toComposeImageBitmap(),native) }
        catch (failure: Throwable) { native.close(); throw failure }
    } finally { image.flush() }
}

private fun frameArgb(bytes: ByteArray, offset: Int): Int =
    ((bytes[offset+3].toInt() and 255) shl 24) or ((bytes[offset].toInt() and 255) shl 16) or
        ((bytes[offset+1].toInt() and 255) shl 8) or (bytes[offset+2].toInt() and 255)

/** Fixed in-memory encoder cap; never changes ImageIO's global cache policy. */
private class FramePngBuffer : OutputStream() {
    private val buffer = ByteArray(2*1024*1024)
    private var size = 0
    override fun write(value: Int) { check(size < buffer.size); buffer[size++] = value.toByte() }
    override fun write(bytes: ByteArray, offset: Int, length: Int) {
        require(offset >= 0 && length >= 0 && offset <= bytes.size-length && length <= buffer.size-size)
        bytes.copyInto(buffer,size,offset,offset+length); size += length
    }
    fun bytes(): ByteArray = buffer.copyOf(size)
}
