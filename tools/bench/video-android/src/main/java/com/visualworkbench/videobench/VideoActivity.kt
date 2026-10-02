package com.visualworkbench.videobench

import android.app.Activity
import android.content.Intent
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.RandomAccessFile
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.ceil

/** Diagnostic activity: accepts only bounded generated fixture files in its own run directory. */
class VideoActivity : Activity(), SurfaceHolder.Callback {
    private data class RenderedFrame(val surfaceTimestamp: Long, val callbackTimestamp: Long)
    private lateinit var surface: SurfaceView
    private lateinit var label: TextView
    private val busy = AtomicBoolean(false)
    private val cancelled = AtomicBoolean(false)
    private var surfaceReady = false
    private var pending: Intent? = null
    @Volatile private var phase = "initialization"

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val layout = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        label = TextView(this).apply { text = "Visual Workbench · generated video benchmark"; textSize = 18f }
        surface = SurfaceView(this)
        layout.addView(label)
        layout.addView(surface, LinearLayout.LayoutParams(-1, 0, 1f))
        setContentView(layout)
        surface.holder.addCallback(this)
        accept(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        accept(intent)
    }

    private fun accept(request: Intent) {
        val run = request.getStringExtra("run_id") ?: return
        if (!run.matches(Regex("[0-9a-f]{32}"))) return
        when (request.getStringExtra("mode")) {
            "prepare" -> {
                val directory = File(requireNotNull(getExternalFilesDir(null)), "vw-video-$run")
                require(!directory.exists() && directory.mkdir())
                File(filesDir, "ready-$run.json").writeText(JSONObject().put("ready", true).toString())
                label.text = "Ready for generated HEVC fixtures"
            }
            "decode" -> {
                if (request.getStringExtra("fixture") !in listOf("portrait", "4k")) return
                pending = request
                startPending()
            }
            "tiles" -> {
                if (request.getStringExtra("fixture") !in listOf("tiles-10", "tiles-25")) return
                if (request.getIntExtra("port", -1) !in 1024..65535) return
                pending = request
                startPending()
            }
        }
    }

    override fun surfaceCreated(holder: SurfaceHolder) { surfaceReady = true; startPending() }
    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) = Unit
    override fun surfaceDestroyed(holder: SurfaceHolder) { surfaceReady = false; cancelled.set(true) }
    override fun onDestroy() { cancelled.set(true); super.onDestroy() }

    private fun startPending() {
        val request = pending ?: return
        if (!surfaceReady || !busy.compareAndSet(false, true)) return
        pending = null
        cancelled.set(false)
        val run = requireNotNull(request.getStringExtra("run_id"))
        val fixture = requireNotNull(request.getStringExtra("fixture"))
        label.text = "Decoding generated $fixture fixture…"
        Thread({
            val result = try {
                if (request.getStringExtra("mode") == "tiles") {
                    phase = "tile_network_decode_surface_post"
                    TileReceiver.receive(surface.holder, request.getIntExtra("port", -1), { !cancelled.get() }) { count, total ->
                        runOnUiThread { label.text = "Posted $count/$total generated JPEG frames" }
                    }
                } else decode(run, fixture)
            } catch (error: Exception) {
                // Exception messages can include file paths. Keep an allowlisted phase/type only.
                JSONObject().put("completed", false).put("error_type", error.javaClass.simpleName)
                    .put("phase", if (cancelled.get()) "surface_lifecycle" else phase)
                    .put("codec_error_code", if (error is MediaCodec.CodecException) error.errorCode else JSONObject.NULL)
            }
            result.put("schema", 1).put("fixture", fixture)
            try {
                val target = File(filesDir, "result-$run-$fixture.json")
                val temporary = File(filesDir, "result-$run-$fixture.tmp")
                temporary.writeText(result.toString(2))
                check(temporary.renameTo(target))
                runOnUiThread { label.text = if (result.optBoolean("completed")) "Decoded $fixture · result saved" else "Decode incomplete · inspect phase receipt" }
            } finally {
                runOnUiThread { busy.set(false); startPending() }
            }
        }, "vw-video-benchmark").start()
    }

    private fun decode(run: String, fixture: String): JSONObject {
        phase = "fixture_validation"
        val root = File(requireNotNull(getExternalFilesDir(null)), "vw-video-$run/$fixture").canonicalFile
        val inputRoot = File(requireNotNull(getExternalFilesDir(null)), "vw-video-$run").canonicalFile
        require(root.parentFile == inputRoot)
        val indexFile = File(root, "index.json")
        require(indexFile.length() in 1..1_048_576)
        val index = JSONObject(indexFile.readText())
        require(index.getBoolean("completed") && index.getString("stream_file") == "stream.h265")
        val width = index.getInt("width")
        val height = index.getInt("height")
        require((fixture == "portrait" && width == 1440 && height == 3088) || (fixture == "4k" && width == 3840 && height == 2160))
        val samples = index.getJSONArray("samples")
        require(samples.length() in 11..250)
        val stream = File(root, "stream.h265")
        require(stream.length() in 1..268_435_456)
        var expectedOffset = 0L
        var previousPts = -1L
        for (i in 0 until samples.length()) {
            val row = samples.getJSONObject(i)
            val length = row.getLong("length")
            val offset = row.getLong("offset")
            val pts = row.getLong("pts_us")
            require(length in 1..33_554_432 && offset == expectedOffset && offset + length <= stream.length())
            require(pts > previousPts && row.getBoolean("warmup") == (i < 10))
            expectedOffset += length
            previousPts = pts
        }
        require(expectedOffset == stream.length())
        phase = "hardware_decoder_selection"
        val info = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.firstOrNull {
            !it.isEncoder && it.isHardwareAccelerated && MediaFormat.MIMETYPE_VIDEO_HEVC in it.supportedTypes &&
                it.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_HEVC).videoCapabilities?.isSizeSupported(width, height) == true
        } ?: error("hardware_decoder_unavailable")
        val caps = info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_HEVC)
        val codec = MediaCodec.createByCodecName(info.name)
        val callbacks = HandlerThread("vw-frame-render-times").apply { start() }
        val submitted = ConcurrentHashMap<Long, Long>()
        val rendered = ConcurrentHashMap<Long, RenderedFrame>()
        val dequeueMs = mutableListOf<Double>()
        val ptsSeen = mutableSetOf<Long>()
        val vendorApplied = mutableListOf<String>()
        val vendorKeys = listOf("vendor.qti-ext-dec-low-latency.enable", "vendor.qti-ext-dec-picture-order.enable")
        var started = false
        try {
            phase = "codec_configuration"
            val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_HEVC, width, height)
            format.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            format.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, samples.let { array -> (0 until array.length()).maxOf { array.getJSONObject(it).getInt("length") } })
            val operatingRate = requireNotNull(caps.videoCapabilities).getSupportedFrameRatesFor(width, height).upper.toFloat()
            require(operatingRate.isFinite() && operatingRate > 0)
            format.setFloat(MediaFormat.KEY_OPERATING_RATE, operatingRate)
            if (android.os.Build.VERSION.SDK_INT >= 31) {
                for (key in vendorKeys) {
                    if (key in codec.supportedVendorParameters && codec.getParameterDescriptor(key)?.type == MediaFormat.TYPE_INTEGER) {
                        format.setInteger(key, 1)
                        vendorApplied.add(key)
                    }
                }
            }
            codec.configure(format, surface.holder.surface, null, 0)
            codec.setOnFrameRenderedListener({ _, pts, nanoTime ->
                if (submitted.containsKey(pts)) rendered.putIfAbsent(pts, RenderedFrame(nanoTime, System.nanoTime()))
            }, Handler(callbacks.looper))
            codec.start()
            started = true
            RandomAccessFile(stream, "r").use { data ->
                val status = MediaCodec.BufferInfo()
                for (i in 0 until samples.length()) {
                    phase = "input_readiness"
                    check(!cancelled.get())
                    val row = samples.getJSONObject(i)
                    val pts = row.getLong("pts_us")
                    val length = row.getInt("length")
                    val input = codec.dequeueInputBuffer(2_000_000)
                    check(input >= 0)
                    val buffer = requireNotNull(codec.getInputBuffer(input))
                    check(buffer.capacity() >= length)
                    val bytes = ByteArray(length)
                    data.seek(row.getLong("offset")); data.readFully(bytes)
                    buffer.clear(); buffer.put(bytes)
                    val queued = System.nanoTime()
                    submitted[pts] = queued
                    phase = "queue_sample"
                    codec.queueInputBuffer(input, 0, length, pts, 0)
                    val deadline = SystemClock.elapsedRealtime() + 3000
                    var received = false
                    phase = "output_wait"
                    while (!received && SystemClock.elapsedRealtime() < deadline) {
                        check(!cancelled.get())
                        val output = codec.dequeueOutputBuffer(status, 10_000)
                        if (output >= 0) {
                            val now = System.nanoTime()
                            val config = status.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                            if (config) { codec.releaseOutputBuffer(output, false); continue }
                            check(status.presentationTimeUs == pts && ptsSeen.add(pts))
                            if (!row.getBoolean("warmup")) dequeueMs.add((now - queued) / 1e6)
                            // Media-relative PTS is not the Surface's monotonic
                            // clock. Schedule immediate rendering explicitly.
                            codec.releaseOutputBuffer(output, System.nanoTime())
                            received = true
                        } else check(output == MediaCodec.INFO_TRY_AGAIN_LATER || output == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED)
                    }
                    check(received)
                    // Release is paced for SurfaceView presentation; decoding latency excludes this wait.
                    Thread.sleep(34)
                    if (i % 15 == 0) runOnUiThread { label.text = "Decoded ${i + 1}/${samples.length()} generated $fixture frames" }
                }
            }
            val deadline = SystemClock.elapsedRealtime() + 2000
            phase = "render_callback_wait"
            while (rendered.size < submitted.size && SystemClock.elapsedRealtime() < deadline) { check(!cancelled.get()); Thread.sleep(10) }
            require(ptsSeen.size == samples.length() && dequeueMs.size == samples.length() - 10)
            val nativeRenderMs = (10 until samples.length()).mapNotNull { i ->
                val pts = samples.getJSONObject(i).getLong("pts_us")
                val time = rendered[pts] ?: return@mapNotNull null
                val queued = requireNotNull(submitted[pts])
                (time.surfaceTimestamp - queued) / 1e6
            }
            val renderMs = (10 until samples.length()).mapNotNull { i ->
                val pts = samples.getJSONObject(i).getLong("pts_us")
                val time = rendered[pts] ?: return@mapNotNull null
                (time.callbackTimestamp - requireNotNull(submitted[pts])) / 1e6
            }
            return JSONObject().put("completed", rendered.size == submitted.size).put("decode_samples_completed", true).put("decoder", info.name)
                .put("hardware_accelerated", info.isHardwareAccelerated)
                .put("low_latency_feature_declared", caps.isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency))
                .put("low_latency_requested", true).put("operating_rate_requested", operatingRate)
                .put("surface_timestamp_mode", "explicit System.nanoTime at output release")
                .put("vendor_parameters_requested", JSONArray(vendorApplied)).put("width", width).put("height", height)
                .put("input_frames", samples.length()).put("output_frames", ptsSeen.size).put("warmup_frames", 10)
                .put("queue_to_output_ms", stats(dequeueMs)).put("raw_queue_to_output_ms", JSONArray(dequeueMs))
                .put("render_callbacks", rendered.size).put("all_render_callbacks_observed", rendered.size == submitted.size)
                .put("queue_to_render_callback_ms", if (renderMs.isEmpty()) JSONObject.NULL else stats(renderMs))
                .put("raw_native_render_timestamp_delta_ms", JSONArray(nativeRenderMs))
                .put("native_render_timestamps_before_queue", nativeRenderMs.count { it < 0 })
                .put("native_render_timing_valid", nativeRenderMs.size == samples.length() - 10 && nativeRenderMs.all { it >= 0 })
                .put("queue_to_native_render_timestamp_ms", if (nativeRenderMs.isNotEmpty() && nativeRenderMs.all { it >= 0 }) stats(nativeRenderMs) else JSONObject.NULL)
                .put("timing_scope", "Queue/dequeue and callback delivery use System.nanoTime on phone; native Surface timestamps are separately validated and retained even when negative; neither proves photon or cross-device latency")
        } finally {
            if (started) runCatching { codec.stop() }
            try { codec.release() } finally {
                callbacks.quitSafely()
                callbacks.join(3000)
            }
        }
    }

    private fun stats(values: List<Double>): JSONObject {
        require(values.isNotEmpty() && values.all { it.isFinite() && it >= 0 })
        val sorted = values.sorted()
        fun at(p: Double) = sorted[ceil(p * sorted.size).toInt() - 1]
        return JSONObject().put("count", values.size).put("p50", at(0.5)).put("p95", at(0.95)).put("max", sorted.last())
    }
}
