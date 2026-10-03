package com.visualworkbench.android.session

import android.content.Context
import android.content.ContextWrapper
import android.util.Size
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.LifecycleOwner
import com.google.zxing.*
import com.google.zxing.common.HybridBinarizer
import com.visualworkbench.shared.decodePairingQr
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Frames are transient analysis input: one worker/latest image, no disk/logs. */
@Composable
internal fun QrScanner(modifier: Modifier, onQr: (ByteArray) -> Unit, onUnavailable: () -> Unit) {
    val context = LocalContext.current
    val view = remember { PreviewView(context) }
    val receive by rememberUpdatedState(onQr)
    val unavailable by rememberUpdatedState(onUnavailable)
    DisposableEffect(view) {
        val active = AtomicBoolean(true)
        val delivered = AtomicBoolean(false)
        val worker = Executors.newSingleThreadExecutor()
        val future = ProcessCameraProvider.getInstance(context)
        var provider: ProcessCameraProvider? = null
        var preview: Preview? = null
        var analysis: ImageAnalysis? = null
        future.addListener({
            if (active.get()) try {
                val camera = future.get()
                provider = camera
                val surface = Preview.Builder().build().also { it.setSurfaceProvider(view.surfaceProvider) }
                @Suppress("DEPRECATION")
                val frames = ImageAnalysis.Builder().setTargetResolution(Size(1280, 720))
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
                preview = surface; analysis = frames
                val decoder = MultiFormatReader().apply { setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE))) }
                frames.setAnalyzer(worker) { image ->
                    try {
                        if (!active.get() || delivered.get()) return@setAnalyzer
                        val crop = image.cropRect
                        val width = crop.width(); val height = crop.height()
                        if (width <= 0 || height <= 0 || width.toLong() * height > 2_000_000) return@setAnalyzer
                        val plane = image.planes.firstOrNull() ?: return@setAnalyzer
                        val buffer = plane.buffer.duplicate()
                        val gray = ByteArray(width * height)
                        try {
                            for (y in 0 until height) for (x in 0 until width) {
                                val index = (crop.top + y).toLong() * plane.rowStride + (crop.left + x).toLong() * plane.pixelStride
                                if (index < 0 || index >= buffer.limit()) return@setAnalyzer
                                gray[y * width + x] = buffer.get(index.toInt())
                            }
                            val result = decoder.decodeWithState(BinaryBitmap(HybridBinarizer(PlanarYUVLuminanceSource(gray, width, height, 0, 0, width, height, false))))
                            if (result.text.length > 5500) return@setAnalyzer
                            val bytes = decodePairingQr(result.text)
                            if (!active.get() || !delivered.compareAndSet(false, true)) bytes.fill(0)
                            else context.mainExecutor.execute { if (active.get()) receive(bytes) else bytes.fill(0) }
                        } finally { gray.fill(0); decoder.reset() }
                    } catch (_: ReaderException) { /* No QR in this frame. */ }
                    catch (_: Exception) { /* Invalid/non-workbench QR never leaves the scanner. */ }
                    finally { image.close() }
                }
                camera.bindToLifecycle(lifecycleOwner(context), CameraSelector.DEFAULT_BACK_CAMERA, surface, frames)
            } catch (_: Exception) { if (active.get()) unavailable() }
        }, context.mainExecutor)
        onDispose {
            active.set(false); analysis?.clearAnalyzer()
            val owned = listOfNotNull(preview, analysis).toTypedArray()
            if (owned.isNotEmpty()) provider?.unbind(*owned)
            worker.shutdown()
        }
    }
    AndroidView(factory = { view }, modifier = modifier)
}

private fun lifecycleOwner(context: Context): LifecycleOwner {
    var current = context
    repeat(16) {
        if (current is LifecycleOwner) return current as LifecycleOwner
        current = (current as? ContextWrapper)?.baseContext ?: error("Camera lifecycle unavailable")
    }
    error("Camera lifecycle unavailable")
}
