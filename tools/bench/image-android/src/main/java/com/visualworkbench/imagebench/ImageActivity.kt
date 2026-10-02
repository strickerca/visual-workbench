package com.visualworkbench.imagebench

import android.app.Activity
import android.app.ActivityManager
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.BitmapRegionDecoder
import android.graphics.Color
import android.graphics.ImageDecoder
import android.graphics.Rect
import android.os.Bundle
import android.os.Debug
import android.os.SystemClock
import android.view.WindowManager
import android.widget.TextView
import androidx.heifwriter.HeifWriter
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import org.json.JSONArray
import org.json.JSONObject

/** Diagnostic APK only: largeHeap is intentional; this is not the product memory budget. */
class ImageActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val busy = AtomicBoolean(false)
    private lateinit var status: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        status = TextView(this).apply { textSize = 22f; text = "Workbench image diagnostic" }
        setContentView(status)
        dispatch(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        dispatch(intent)
    }

    override fun onDestroy() {
        worker.shutdownNow()
        super.onDestroy()
    }

    private fun write(name: String, value: JSONObject) {
        val temporary = File(filesDir, "$name.tmp")
        temporary.writeText(value.toString())
        check(temporary.renameTo(File(filesDir, name))) { "Receipt rename failed" }
    }

    private fun dispatch(request: Intent) {
        val run = request.getStringExtra("run_id") ?: return
        val mode = request.getStringExtra("mode") ?: return
        if (!run.matches(Regex("[0-9a-f]{32}")) || mode !in listOf("prepare", "jpeg", "generate", "heif")) return
        if (!busy.compareAndSet(false, true)) return
        val expected = request.getStringExtra("sha256")
        worker.execute {
            val result = JSONObject().put("schema", 1).put("run_id", run).put("mode", mode)
                .put("completed", false).put("measurements", JSONArray()).put("large_heap_diagnostic", true)
                .put("max_java_heap_bytes", Runtime.getRuntime().maxMemory())
            fun phase(value: String) {
                result.put("last_phase", value)
                write("progress-$run.json", JSONObject().put("phase", value).put("uptime_ms", SystemClock.elapsedRealtime()))
                runOnUiThread { status.text = "Image diagnostic: $value" }
            }
            try {
                val directory = File(checkNotNull(getExternalFilesDir(null)), "vw-image-$run")
                if (mode == "prepare") {
                    check(directory.mkdir()) { "Expected a fresh fixture directory" }
                } else {
                    check(directory.isDirectory) { "Prepare fixture directory first" }
                    when (mode) {
                        "jpeg" -> {
                            val source = File(directory, "200mp.jpg")
                            phase("jpeg-hash")
                            check(expected?.matches(Regex("[0-9a-f]{64}")) == true && hash(source) == expected) { "JPEG binding differs" }
                            result.put("input_sha256", expected)
                            jpeg(source, result, ::phase)
                        }
                        "generate" -> generate(File(directory, "200mp.heic"), result, ::phase)
                        "heif" -> {
                            val source = File(directory, "200mp.heic")
                            phase("heif-hash")
                            check(expected?.matches(Regex("[0-9a-f]{64}")) == true && hash(source) == expected) { "HEIF binding differs" }
                            result.put("input_sha256", expected)
                            heif(source, result, ::phase)
                        }
                    }
                }
                result.put("completed", true)
            } catch (error: Exception) {
                result.put("error_type", error.javaClass.simpleName)
            } catch (_: OutOfMemoryError) {
                // Preserve a bounded failure receipt rather than claim a decoded image.
                result.put("error_type", "OutOfMemoryError")
            } finally {
                write("result-$run-$mode.json", result)
                phase(if (result.optBoolean("completed")) "done-$mode" else "failed-$mode")
                busy.set(false)
            }
        }
    }

    private fun hash(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().buffered().use { stream ->
            val block = ByteArray(65536)
            while (true) {
                val read = stream.read(block)
                if (read < 0) break
                digest.update(block, 0, read)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it.toInt() and 255) }
    }

    private fun measure(name: String, result: JSONObject, operation: () -> Bitmap) {
        val baseline = Debug.getPss()
        var peak = baseline
        val sampling = AtomicBoolean(true)
        val sampler = thread(name = "image-memory-sampler") {
            while (sampling.get()) {
                peak = maxOf(peak, Debug.getPss())
                Thread.sleep(100)
            }
        }
        var bitmap: Bitmap? = null
        try {
            val start = SystemClock.elapsedRealtimeNanos()
            bitmap = operation()
            val elapsed = (SystemClock.elapsedRealtimeNanos() - start) / 1_000_000.0
            sampling.set(false)
            sampler.join()
            peak = maxOf(peak, Debug.getPss())
            result.getJSONArray("measurements").put(JSONObject().put("operation", name)
                .put("elapsed_ms", elapsed).put("width", bitmap.width).put("height", bitmap.height)
                .put("bitmap_allocation_bytes", bitmap.allocationByteCount)
                .put("pss_baseline_kib", baseline).put("pss_sampled_peak_kib", peak)
                .put("memory_sampling_interval_ms", 100)
                .put("center_pixel", bitmap.getPixel(bitmap.width / 2, bitmap.height / 2)))
        } finally {
            sampling.set(false)
            sampler.join()
            bitmap?.recycle()
        }
    }

    @Suppress("DEPRECATION")
    private fun jpeg(source: File, result: JSONObject, phase: (String) -> Unit) {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(source.path, bounds)
        check(bounds.outWidth == 16320 && bounds.outHeight == 12240) { "JPEG dimensions differ" }
        for (attempt in 0..2) {
            for ((label, top) in listOf("top" to 0, "middle" to 5608, "bottom" to 11216)) {
                val name = "jpeg-region-$label-$attempt"
                phase(name)
                measure(name, result) {
                    // Include opening the region decoder in each measurement.
                    val decoder = checkNotNull(BitmapRegionDecoder.newInstance(source.path, false))
                    try {
                        checkNotNull(decoder.decodeRegion(Rect(7648, top, 8672, top + 1024),
                            BitmapFactory.Options().apply { inPreferredConfig = Bitmap.Config.ARGB_8888 }))
                            .also { check(it.width == 1024 && it.height == 1024) }
                    } finally { decoder.recycle() }
                }
            }
            phase("jpeg-sample8-$attempt")
            measure("jpeg-sample8-$attempt", result) {
                checkNotNull(BitmapFactory.decodeFile(source.path, BitmapFactory.Options().apply {
                    inSampleSize = 8; inPreferredConfig = Bitmap.Config.ARGB_8888
                })).also { check(it.width == 2040 && it.height == 1530) }
            }
        }
    }

    private fun available(): Long {
        val memory = ActivityManager.MemoryInfo()
        getSystemService(ActivityManager::class.java).getMemoryInfo(memory)
        return memory.availMem
    }

    private fun generate(output: File, result: JSONObject, phase: (String) -> Unit) {
        val width = 16320
        val height = 12240
        val patternWidth = 2040
        val patternHeight = 1530
        val inputBytes = patternWidth * patternHeight * 4
        val runtime = Runtime.getRuntime()
        val freeHeap = runtime.maxMemory() - (runtime.totalMemory() - runtime.freeMemory())
        val availableSystem = available()
        result.put("input_bytes", inputBytes).put("generation_strategy", "small_bitmap_scaled_by_writer_grid")
            .put("pattern_width", patternWidth).put("pattern_height", patternHeight)
            .put("available_system_bytes", availableSystem).put("available_java_heap_bytes", freeHeap)
        check(freeHeap > inputBytes + 64L * 1024 * 1024 && availableSystem > inputBytes.toLong() * 2 + 512L * 1024 * 1024) {
            "HEIF input memory guard rejected allocation"
        }
        phase("heif-pattern-allocation")
        val bitmap = Bitmap.createBitmap(patternWidth, patternHeight, Bitmap.Config.ARGB_8888)
        try {
            val row = IntArray(patternWidth)
            for (y in 0 until patternHeight) {
                for (x in row.indices) row[x] = Color.rgb(16 + x * 220 / patternWidth, 16 + y * 220 / patternHeight, 80)
                bitmap.setPixels(row, 0, patternWidth, 0, y, patternWidth, 1)
            }
            phase("heif-writer-build")
            // HeifWriter 1.1.0 normalizes tile source coordinates against the configured
            // output dimensions. A small texture is scaled over that grid, avoiding
            // the buffer mode's three full-size YUV allocations. This fixture has
            // 200 MP dimensions, not 200 MP independent source detail.
            val writer = HeifWriter.Builder(output.path, width, height, HeifWriter.INPUT_MODE_BITMAP)
                .setQuality(95).setGridEnabled(true).setMaxImages(1).build()
            // Use the public lifetime contract. In 1.1.0 the inherited close
            // implementation is declared on the library-restricted WriterBase.
            val lifetime: AutoCloseable = writer
            try {
                phase("heif-writer-encode")
                val start = SystemClock.elapsedRealtimeNanos()
                writer.start()
                writer.addBitmap(bitmap)
                writer.stop(120_000)
                result.put("encode_ms", (SystemClock.elapsedRealtimeNanos() - start) / 1_000_000.0)
            } finally { lifetime.close() }
        } finally { bitmap.recycle() }
        phase("heif-generated-hash")
        result.put("output_sha256", hash(output)).put("output_bytes", output.length())
            .put("width", width).put("height", height).put("pattern", "2040x1530 RGB: (16+x*220/2040,16+y*220/1530,80), scaled to 16320x12240")
    }

    private fun heif(source: File, result: JSONObject, phase: (String) -> Unit) {
        for (attempt in 0..2) {
            phase("heif-target-$attempt")
            measure("heif-target-$attempt", result) {
                ImageDecoder.decodeBitmap(ImageDecoder.createSource(source)) { decoder, info, _ ->
                    check(info.size.width == 16320 && info.size.height == 12240) { "HEIF dimensions differ" }
                    decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                    decoder.setTargetSize(2040, 1530)
                }.also {
                    check(it.width == 2040 && it.height == 1530)
                    // Catch empty tiles, flips and unexpected bitmap-input scaling.
                    for ((x, y) in listOf(255 to 191, 1785 to 1339, 1020 to 765)) {
                        val pixel = it.getPixel(x, y)
                        check(kotlin.math.abs(Color.red(pixel) - (16 + x * 220 / 2040)) <= 12 &&
                            kotlin.math.abs(Color.green(pixel) - (16 + y * 220 / 1530)) <= 12 &&
                            kotlin.math.abs(Color.blue(pixel) - 80) <= 12) { "HEIF pattern content differs" }
                    }
                }
            }
        }
        val required = 16320L * 12240 * 4
        val maximum = minOf(Runtime.getRuntime().maxMemory() * 3 / 4, available() / 3)
        result.put("full_decode_estimate_bytes", required).put("full_decode_budget_bytes", maximum)
        if (required > maximum) {
            result.put("full_decode_status", "skipped_memory_guard")
        } else {
            phase("heif-full")
            measure("heif-full", result) {
                ImageDecoder.decodeBitmap(ImageDecoder.createSource(source)) { decoder, _, _ ->
                    decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                }.also { check(it.width == 16320 && it.height == 12240) }
            }
            result.put("full_decode_status", "decoded")
        }
    }
}
