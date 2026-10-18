package com.visualworkbench.android.session

import android.content.Context
import android.content.ContextWrapper
import android.util.Size
import android.util.Log
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
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

private const val QR_SCANNER_TAG = "VWPairingScanner"

/** Frames are transient analysis input: one worker/latest image, no disk/logs. */
@Composable
internal fun QrScanner(modifier: Modifier, onQr: (ByteArray) -> Boolean, onUnavailable: () -> Unit) {
    val context = LocalContext.current
    val view = remember { PreviewView(context) }
    val receive by rememberUpdatedState(onQr)
    val unavailable by rememberUpdatedState(onUnavailable)
    DisposableEffect(view) {
        val active = AtomicBoolean(true)
        val delivered = AtomicBoolean(false)
        val frameSeen = AtomicBoolean(false)
        val luminanceSeen = AtomicBoolean(false)
        val decodedSeen = AtomicBoolean(false)
        val acceptedSeen = AtomicBoolean(false)
        val refusedSeen = AtomicBoolean(false)
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
                val resolution = ResolutionSelector.Builder().setResolutionStrategy(
                    ResolutionStrategy(Size(1280, 720), ResolutionStrategy.FALLBACK_RULE_CLOSEST_LOWER_THEN_HIGHER)
                ).build()
                val frames = ImageAnalysis.Builder().setResolutionSelector(resolution)
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
                preview = surface; analysis = frames
                val decoder = MultiFormatReader().apply { setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE))) }
                frames.setAnalyzer(worker) { image ->
                    try {
                        if (!active.get() || delivered.get()) return@setAnalyzer
                        if (frameSeen.compareAndSet(false, true)) Log.d(QR_SCANNER_TAG, "analysis_frame_seen")
                        val crop = image.cropRect
                        val plane = image.planes.firstOrNull() ?: return@setAnalyzer
                        val sampled = sampleQrLuminance(plane.buffer, plane.rowStride, plane.pixelStride,
                            image.width, image.height, crop.left, crop.top, crop.width(), crop.height()) ?: return@setAnalyzer
                        if (luminanceSeen.compareAndSet(false, true)) Log.d(QR_SCANNER_TAG,
                            "luminance_ready input=${image.width}x${image.height} crop=${crop.width()}x${crop.height()} sample=${sampled.width}x${sampled.height}")
                        val width = sampled.width; val height = sampled.height
                        val gray = sampled.bytes
                        try {
                            val result = decoder.decodeWithState(BinaryBitmap(HybridBinarizer(PlanarYUVLuminanceSource(gray, width, height, 0, 0, width, height, false))))
                            if (result.text.length > 5500) return@setAnalyzer
                            val bytes = decodePairingQr(result.text)
                            if (decodedSeen.compareAndSet(false, true)) Log.d(QR_SCANNER_TAG, "pairing_qr_decoded")
                            if (!active.get() || !delivered.compareAndSet(false, true)) bytes.fill(0)
                            else try {
                                context.mainExecutor.execute {
                                    var accepted = false
                                    try { if (active.get()) accepted = receive(bytes) }
                                    finally {
                                        bytes.fill(0)
                                        val marker = if (accepted) acceptedSeen else refusedSeen
                                        if (marker.compareAndSet(false, true)) Log.d(QR_SCANNER_TAG,
                                            if (accepted) "pairing_qr_handoff_accepted" else "pairing_qr_handoff_refused")
                                        // A transient owner/epoch/busy refusal is not a delivered QR.
                                        if (!accepted) delivered.set(false)
                                    }
                                }
                            } catch (error: Exception) {
                                bytes.fill(0); delivered.set(false)
                                throw error
                            }
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
