package com.visualworkbench.desktop.capture

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.*
import androidx.compose.ui.window.*
import com.visualworkbench.bindings.host.*
import com.visualworkbench.shared.Camera
import com.visualworkbench.shared.Point
import kotlinx.coroutines.*
import org.jetbrains.skia.Image
import java.awt.image.BufferedImage
import java.util.concurrent.atomic.AtomicReference
import javax.imageio.stream.MemoryCacheImageOutputStream
import javax.imageio.ImageIO

/** Secondary window is excluded before becoming visible. All catalog labels
 * are literal UI text; none are paths, selectors, commands or semantic input. */
@Composable internal fun CapturePickerWindow(controller: CapturePickerController, report: (String) -> Unit) {
    val state by controller.state.collectAsState()
    if (!state.open) return
    val scope = rememberCoroutineScope()
    var excluded by remember { mutableStateOf(false) }
    fun closePicker() { scope.launch {
        try { controller.dismiss() }
        catch (error: CancellationException) { throw error }
        catch (_: Exception) { report("Capture cleanup is pending. Retry closing before leaving the editor.") }
    } }
    DialogWindow(onCloseRequest = ::closePicker, visible = excluded,
        state = rememberDialogState(width = 1100.dp, height = 740.dp), title = "Capture source") {
        LaunchedEffect(window) {
            try {
                excludeCaptureWindows(listOf(window.windowHandle.toULong()))
                currentCoroutineContext().ensureActive()
                excluded = true
            }
            catch (error: CancellationException) { throw error }
            catch (_: Exception) {
                report("The picker window could not be excluded from screen capture.")
                closePicker()
            }
        }
        MaterialTheme(colors = darkColors()) {
            Surface(Modifier.fillMaxSize()) { Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("Choose a source", style = MaterialTheme.typography.h6)
                Text("A selection captures one lossless SDR snapshot. Originals keep physical pixels; the preview is only for selection.", style = MaterialTheme.typography.body2)
                Row(Modifier.weight(1f), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Column(Modifier.width(320.dp).fillMaxHeight().verticalScroll(rememberScrollState())) {
                        TextButton(onClick = controller::refresh, enabled = !state.busy && !state.cleanupPending) { Text("Refresh sources") }
                        Text("Monitors", style = MaterialTheme.typography.subtitle1)
                        state.catalog?.monitors?.forEachIndexed { index, monitor ->
                            Text("Monitor ${index + 1}${if (monitor.primary) " · primary" else ""} · ${monitor.width} × ${monitor.height} · ${monitor.dpi} DPI", style = MaterialTheme.typography.body2)
                            Row { TextButton(onClick = { controller.select(CaptureSelection.Monitor(monitor)) }, enabled = !state.busy && !state.cleanupPending && !state.regionOnly) { Text("Whole monitor") }
                                TextButton(onClick = { controller.select(CaptureSelection.Monitor(monitor), true) }, enabled = !state.busy && !state.cleanupPending) { Text("Draw region") } }
                        }
                        Divider(); Text("Windows", style = MaterialTheme.typography.subtitle1)
                        state.catalog?.windows?.forEach { entry ->
                            TextButton(onClick = { controller.select(CaptureSelection.Window(entry.target)) }, enabled = !state.busy && !state.cleanupPending && !state.regionOnly, modifier = Modifier.fillMaxWidth()) {
                                Text(entry.title.ifBlank { "Untitled window" }, maxLines = 3)
                            }
                        }
                    }
                    Box(Modifier.weight(1f).fillMaxHeight().background(Color(0xff101820))) {
                        state.preview?.let { CapturePreview(controller, it, state) }
                    }
                }
                state.message?.let { Text(it, style = MaterialTheme.typography.body2) }
                if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Button(onClick = { controller.save(state.preview) }, enabled = !state.busy && state.acceptsPreview(state.preview) && (!state.region || state.regionReady)) { Text("Save exact snapshot") }
                    Button(onClick={controller.shareLive(state.preview)},enabled=state.canShareLive&&!state.busy&&state.acceptsPreview(state.preview)&&(!state.region||state.regionReady)){Text("Share live with paired device")}
                    TextButton(onClick = controller::cancelWork, enabled = state.busy) { Text("Cancel operation") }
                    TextButton(onClick = ::closePicker) { Text("Discard preview and close") }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    var shortcut by remember { mutableStateOf("R") }
                    OutlinedTextField(shortcut, { shortcut = it.uppercase().filter { c -> c in 'A'..'Z' }.take(1) },
                        enabled = !state.busy && !state.cleanupPending && !state.hotkeyEnabled, label = { Text("Ctrl+Alt+letter") }, modifier = Modifier.width(180.dp))
                    TextButton(onClick = { if (state.hotkeyEnabled) controller.disableRegionHotkey() else controller.enableRegionHotkey(3u, shortcut.single().code.toUInt()) },
                        enabled = !state.busy && !state.cleanupPending && shortcut.length == 1) { Text(if (state.hotkeyEnabled) "Disable region hotkey" else "Enable region picker hotkey") }
                }
            } }
        }
    }
}

@Composable private fun CapturePreview(controller: CapturePickerController, source: PickerPreview, state: CapturePickerState) {
    var viewport by remember { mutableStateOf(IntSize.Zero) }
    var bitmap by remember(source) { mutableStateOf<PickerBitmap?>(null) }
    val firstDraw = remember(source) { CompletableDeferred<Unit>() }
    LaunchedEffect(source) {
        try { bitmap = pickerReadBitmap(source) }
        catch (error: CancellationException) { throw error }
        catch (_: Exception) { controller.previewUnavailable(source) }
    }
    LaunchedEffect(source, firstDraw) {
        firstDraw.await()
        withFrameNanos { }
        controller.previewPresented(source)
    }
    val camera = remember(source, viewport) {
        Camera(Point(source.sourceWidth.toDouble() / 2, source.sourceHeight.toDouble() / 2),
            minOf(viewport.width.toDouble() / source.sourceWidth.toDouble(), viewport.height.toDouble() / source.sourceHeight.toDouble()).coerceAtLeast(.0001),
            0.0, viewport.width.toDouble().coerceAtLeast(1.0), viewport.height.toDouble().coerceAtLeast(1.0))
    }
    LaunchedEffect(source, camera) { controller.bindRegion(source, camera) }
    Canvas(Modifier.fillMaxSize().onSizeChanged { viewport = it }.pointerInput(source, camera, state.region, state.busy, state.previewReady) {
        try { awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            if (!state.region || state.busy || !state.acceptsPreview(source)) return@awaitEachGesture
            controller.regionBegin(Point(down.position.x.toDouble(), down.position.y.toDouble()), source, camera); down.consume()
            var valid = true
            while (true) {
                val event = awaitPointerEvent(); val change = event.changes.firstOrNull { it.id == down.id }
                if (change == null || event.changes.count { it.pressed } > 1) { valid = false; controller.regionCancel(source, camera); break }
                val point = Point(change.position.x.toDouble(), change.position.y.toDouble())
                if (!change.pressed) { controller.regionFinish(point, source, camera); change.consume(); break }
                controller.regionMove(point, source, camera); change.consume()
            }
            if (!valid) controller.regionCancel(source, camera)
        } } finally { if (controller.region.contacting) controller.regionCancel(source, camera) }
    }) {
        val display = pickerDisplayTransform(source, camera)
        bitmap?.let { image -> withTransform({
            translate(display.left, display.top)
            scale(display.scaleX, display.scaleY, pivot = Offset.Zero)
        }) { drawImage(image.image, filterQuality = FilterQuality.None) }; firstDraw.complete(Unit) }
        state.regionEdges?.let { edges ->
            val points = controller.core.mapPoints(camera, false, listOf(Point(edges[0], edges[1]), Point(edges[2], edges[3])))
            drawRect(Color(0xffffce70), Offset(minOf(points[0].x, points[1].x).toFloat(), minOf(points[0].y, points[1].y).toFloat()),
                Size(kotlin.math.abs(points[0].x-points[1].x).toFloat(), kotlin.math.abs(points[0].y-points[1].y).toFloat()), style = Stroke(2f))
        }
    }
}

/** Unpublished native images have explicit ownership. Published images use the
 * same Compose/Skia render lifetime as the editor's existing AiBitmap path. */
private class PickerBitmap(val image: ImageBitmap, private val native: Image) {
    fun abandon() { native.close() }
}
private suspend fun pickerReadBitmap(source: PickerPreview): PickerBitmap {
    val handoff = AtomicReference<PickerBitmap?>(null)
    try {
        val value = withContext(Dispatchers.Default) { pickerBitmap(source).also(handoff::set) }
        currentCoroutineContext().ensureActive()
        handoff.set(null)
        return value
    } finally { handoff.getAndSet(null)?.abandon() }
}
private suspend fun pickerBitmap(source: PickerPreview): PickerBitmap {
    currentCoroutineContext().ensureActive()
    val pixels = pickerArgb(source)
    val image = BufferedImage(source.width.toInt(), source.height.toInt(), BufferedImage.TYPE_INT_ARGB)
    image.setRGB(0, 0, image.width, image.height, pixels, 0, image.width)
    val encoded = PickerPngBuffer()
    // Explicit memory stream: no ImageIO global cache setting or ambient file.
    MemoryCacheImageOutputStream(encoded).use { check(ImageIO.write(image, "png", it)) }
    currentCoroutineContext().ensureActive()
    val native = Image.makeFromEncoded(encoded.bytes())
    try {
        currentCoroutineContext().ensureActive()
        return PickerBitmap(native.toComposeImageBitmap(), native)
    } catch (failure: Throwable) { native.close(); throw failure }
}
