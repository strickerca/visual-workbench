package com.visualworkbench.shared.giant

import android.graphics.*
import android.os.Build
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.min

/** App-private source copy only. The native owner validates original BLAKE3 and
 * header metadata before invoking this provider and again before cache adoption.
 * This adapter never accepts a content URI or rewrites the immutable original. */
public data class JpegTileInput(
    public val path: String, public val sha256: String,
    public val rawWidth: Int, public val rawHeight: Int, public val orientation: Int,
    public val encodedBytes: Long,
    public val maxDiskBytes: Long = 3L * 1024 * 1024 * 1024,
    public val maxBandBytes: Long = 32L * 1024 * 1024,
)
public enum class TileBuildPhase { Preview, FullResolution, LowerLevels, Complete }
public data class TileBuildProgress(public val phase: TileBuildPhase, public val completed: Int, public val total: Int)
public class TileBuildFailure(public val reason: String) : Exception(reason)

/** Owns only this operation's exact recorded files. Native adoption copies or
 * moves these files first; caller then closes after the producer has settled. */
public class AndroidTileBuild internal constructor(
    public val directory: File, public val width: Int, public val height: Int,
    public val previewWidth: Int, public val previewHeight: Int,
    private val marker: String, private val files: List<File>,
) {
    private var closed = false
    public suspend fun close(): Unit = withContext(NonCancellable + Dispatchers.IO) { closeNow() }
    @Synchronized
    internal fun closeNow(): Unit {
        if (closed) return
        val owner = File(directory, ".owner")
        if (!directory.isDirectory || directory.canonicalFile != directory.absoluteFile ||
            owner.canonicalFile.parentFile != directory || owner.length() != marker.length.toLong() ||
            owner.readText(Charsets.US_ASCII) != marker) throw TileBuildFailure("cleanup identity")
        for (file in files.asReversed()) {
            if (!file.exists()) continue
            if (file.canonicalFile != file.absoluteFile || !file.isFile ||
                !file.toPath().startsWith(directory.toPath()) || !file.delete()) throw TileBuildFailure("cleanup file")
        }
        (directory.listFiles() ?: throw TileBuildFailure("cleanup inventory")).filter { it.isDirectory }.forEach { dir ->
            if (dir.canonicalFile != dir.absoluteFile || !dir.name.matches(Regex("[LR][0-9]{1,2}")) || !dir.delete()) throw TileBuildFailure("cleanup directory")
        }
        val remaining = directory.listFiles() ?: throw TileBuildFailure("cleanup inventory")
        if (remaining.size != 1 || remaining.single() != owner) throw TileBuildFailure("cleanup foreign entry")
        if (!owner.delete() || !directory.delete()) throw TileBuildFailure("cleanup retained")
        closed = true
    }
}

/** One actual decoder operation per process; cancellation settles the retained
 * producer before releasing its slot or deleting files. BitmapRegionDecoder is
 * not interruptible inside a codec call, so no hard Android cancellation latency
 * or opaque decoder heap claim is made. Every returned bitmap is budget-checked. */
public object AndroidJpegPyramid {
    private val slots = Semaphore(1)
    private val dispatcher by lazy {
        Executors.newSingleThreadExecutor { task -> Thread(task, "vw-jpeg-pyramid").apply { isDaemon = true } }.asCoroutineDispatcher()
    }
    private val producerScope by lazy { CoroutineScope(SupervisorJob() + dispatcher) }

    public suspend fun build(input: JpegTileInput, privateWork: File, progress: (TileBuildProgress) -> Unit): AndroidTileBuild {
        currentCoroutineContext().ensureActive()
        if (!slots.tryAcquire()) throw TileBuildFailure("busy")
        val cancelled = AtomicBoolean(false)
        val result = AtomicReference<AndroidTileBuild?>(null)
        try {
            val producer = producerScope.async { produce(input, privateWork, cancelled, progress).also(result::set) }
            try {
                val value = producer.await()
                currentCoroutineContext().ensureActive()
                result.set(null)
                return value
            } catch (error: CancellationException) {
                cancelled.set(true)
                withContext(NonCancellable) {
                    try { producer.await() } catch (_: Exception) { }
                    result.getAndSet(null)?.close()
                }
                throw error
            }
        } finally { slots.release() }
    }

    private fun produce(input: JpegTileInput, privateWork: File, cancel: AtomicBoolean, progress: (TileBuildProgress) -> Unit): AndroidTileBuild {
        requireInput(input)
        val work = privateWork.canonicalFile
        if (!work.isDirectory || work != privateWork.absoluteFile) throw TileBuildFailure("private work")
        val original = File(input.path)
        if (!original.isFile || original.canonicalFile != original.absoluteFile || original.length() != input.encodedBytes) throw TileBuildFailure("source binding")
        FileInputStream(original).use { locked ->
            locked.channel.lock(0L, Long.MAX_VALUE, true).use {
                if (hash(original, cancel, input.encodedBytes) != input.sha256) throw TileBuildFailure("source hash")
                rejectProfileAndNonJpeg(original)
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(original.path, bounds)
                if (bounds.outWidth != input.rawWidth || bounds.outHeight != input.rawHeight || bounds.outMimeType != "image/jpeg" ||
                    bounds.outColorSpace?.isSrgb == false) throw TileBuildFailure("source metadata")
                val (width, height) = if (input.orientation >= 5) input.rawHeight to input.rawWidth else input.rawWidth to input.rawHeight
                if (width.toLong() * min(256, height) * 4L * 2L > input.maxBandBytes) throw TileBuildFailure("band budget")
                val marker = UUID.randomUUID().toString()
                val directory = File(work, "vw-jpeg-$marker")
                if (!directory.mkdir()) throw TileBuildFailure("staging")
                val files = ArrayList<File>()
                val owner = File(directory, ".owner")
                var previewWidth = 0; var previewHeight = 0
                fun owned(): AndroidTileBuild = AndroidTileBuild(directory, width, height, previewWidth, previewHeight, marker, files.toList())
                try {
                    FileOutputStream(owner).use { it.write(marker.toByteArray(Charsets.US_ASCII)); it.fd.sync() }
                    var disk = marker.length.toLong()
                    fun check(): Unit { if (cancel.get()) throw CancellationException("tile build cancelled") }
                    fun save(bitmap: Bitmap, file: File, lossless: Boolean): Unit {
                        check()
                        if (!file.createNewFile()) throw TileBuildFailure("tile exists")
                        // Track before writing so an encoder exception/cancel still
                        // leaves exact owned cleanup, never recursive deletion.
                        files.add(file)
                        FileOutputStream(file).use { stream ->
                            val fileLimit = if (file.name == "preview.jpg") 16L * 1024 * 1024 else 2L * 1024 * 1024
                            val bounded = CappedOutput(stream, min(fileLimit, input.maxDiskBytes - disk))
                            val format = if (lossless) Bitmap.CompressFormat.WEBP_LOSSLESS else Bitmap.CompressFormat.JPEG
                            if (!bitmap.compress(format, if (lossless) 100 else 90, bounded)) throw TileBuildFailure("tile encode")
                            bounded.flush(); stream.fd.sync(); disk += bounded.written
                        }
                        check()
                    }
                    check()
                    val sample = previewSample(input.rawWidth, input.rawHeight)
                    val preview = BitmapFactory.decodeFile(original.path, options(sample)) ?: throw TileBuildFailure("preview decode")
                    try {
                        admitPreview(preview)
                        val oriented = orient(preview, input.orientation)
                        try { admitPreview(oriented); previewWidth = oriented.width; previewHeight = oriented.height; save(oriented, File(directory, "preview.jpg"), false) }
                        finally { if (oriented !== preview) oriented.recycle() }
                    } finally { preview.recycle() }
                    progress(TileBuildProgress(TileBuildPhase.Preview, 0, 1))
                    val level0 = File(directory, "L0"); if (!level0.mkdir()) throw TileBuildFailure("staging level")
                    @Suppress("DEPRECATION")
                    val decoder = BitmapRegionDecoder.newInstance(original.path, false) ?: throw TileBuildFailure("region decoder")
                    try {
                        var done = 0
                        val total = (width + 255) / 256 * ((height + 255) / 256)
                        for (y in 0 until height step 256) {
                            check()
                            val bandHeight = min(256, height - y)
                            val raw = rawRect(0, y, width, bandHeight, input.rawWidth, input.rawHeight, input.orientation)
                            val decoded = decoder.decodeRegion(raw, options(1)) ?: throw TileBuildFailure("band decode")
                            try {
                                admitBitmap(decoded, input.maxBandBytes / 2)
                                if (decoded.width != raw.width() || decoded.height != raw.height()) throw TileBuildFailure("band extent")
                                val band = orient(decoded, input.orientation)
                                try {
                                    if (band.width != width || band.height != bandHeight) throw TileBuildFailure("oriented band extent")
                                    for (x in 0 until width step 256) {
                                        check(); val tile = Bitmap.createBitmap(band, x, 0, min(256, width - x), bandHeight)
                                        try { save(tile, File(level0, "${x / 256}_${y / 256}.webp"), true) }
                                        finally { if (tile !== band) tile.recycle() }
                                        progress(TileBuildProgress(TileBuildPhase.FullResolution, ++done, total))
                                    }
                                } finally { if (band !== decoded) band.recycle() }
                            } finally { decoded.recycle() }
                        }
                    } finally { decoder.recycle() }
                    var previousWidth = width; var previousHeight = height; var level = 1
                    while (previousWidth != 1 || previousHeight != 1) {
                        check(); val nextWidth = (previousWidth + 1) / 2; val nextHeight = (previousHeight + 1) / 2
                        val target = File(directory, "L$level"); if (!target.mkdir()) throw TileBuildFailure("staging level")
                        val previous = File(directory, if (level == 1) "L0" else "R${level - 1}")
                        val losslessTarget = File(directory, "R$level"); if (!losslessTarget.mkdir()) throw TileBuildFailure("working level")
                        val total = (nextWidth + 255) / 256 * ((nextHeight + 255) / 256); var done = 0
                        for (y in 0 until nextHeight step 256) for (x in 0 until nextWidth step 256) {
                            check()
                            val tile = lowerTile(previous, x, y, nextWidth, nextHeight, previousWidth, previousHeight)
                            try {
                                save(tile, File(losslessTarget, "${x / 256}_${y / 256}.webp"), true)
                                save(tile, File(target, "${x / 256}_${y / 256}.jpg"), false)
                            } finally { tile.recycle() }
                            progress(TileBuildProgress(TileBuildPhase.LowerLevels, ++done, total))
                        }
                        if (level > 1) removeWorking(previous, files)
                        previousWidth = nextWidth; previousHeight = nextHeight; level++
                    }
                    if (level > 1) removeWorking(File(directory, "R${level - 1}"), files)
                    check(); if (hash(original, cancel, input.encodedBytes) != input.sha256 || original.length() != input.encodedBytes) throw TileBuildFailure("source changed")
                    progress(TileBuildProgress(TileBuildPhase.Complete, 1, 1)); return owned()
                } catch (error: Throwable) {
                    try { owned().closeNow() } catch (cleanup: Exception) { error.addSuppressed(cleanup) }
                    throw error
                }
            }
        }
    }

    private fun requireInput(input: JpegTileInput): Unit {
        if (Build.VERSION.SDK_INT < 30 || input.rawWidth !in 1..1_048_576 || input.rawHeight !in 1..1_048_576 ||
            input.rawWidth.toLong() * input.rawHeight > 1_000_000_000L || input.orientation !in 1..8 ||
            input.encodedBytes !in 33..4L * 1024 * 1024 * 1024 || input.maxDiskBytes !in 1..16L * 1024 * 1024 * 1024 ||
            input.maxBandBytes !in 4L * 1024 * 1024..64L * 1024 * 1024 || !input.sha256.matches(Regex("[0-9a-f]{64}"))) throw TileBuildFailure("input limit")
        if (input.rawWidth.toLong() * input.rawHeight * 8L + 64L * 1024 * 1024 > input.maxDiskBytes) throw TileBuildFailure("disk budget")
    }
    internal fun previewSample(rawWidth: Int, rawHeight: Int): Int {
        if (rawWidth !in 1..1_048_576 || rawHeight !in 1..1_048_576) throw TileBuildFailure("preview source extent")
        val longEdge = maxOf(rawWidth, rawHeight).toLong()
        var sample = 1
        // JPEG decoders may round sampled dimensions upward. Admit that upper
        // bound, not integer floor, then verify the actual returned bitmap too.
        while ((longEdge + sample - 1L) / sample > 2048L) sample *= 2
        return sample
    }
    internal fun admitPreview(bitmap: Bitmap): Unit {
        admitBitmap(bitmap, 32L * 1024 * 1024)
        if (bitmap.width <= 0 || bitmap.height <= 0 || maxOf(bitmap.width, bitmap.height) > 2048) throw TileBuildFailure("preview extent")
    }
    private fun options(sample: Int): BitmapFactory.Options = BitmapFactory.Options().apply {
        inSampleSize = sample; inPreferredConfig = Bitmap.Config.ARGB_8888; inScaled = false
        inPreferredColorSpace = ColorSpace.get(ColorSpace.Named.SRGB)
    }
    private fun admitBitmap(bitmap: Bitmap, budget: Long): Unit {
        if (bitmap.config != Bitmap.Config.ARGB_8888 || bitmap.colorSpace?.isSrgb != true || bitmap.allocationByteCount.toLong() > budget) throw TileBuildFailure("decoded bitmap budget")
    }
    private fun hash(file: File, cancel: AtomicBoolean, limit: Long): String {
        val digest = MessageDigest.getInstance("SHA-256"); val chunk = ByteArray(65536); var count = 0L
        FileInputStream(file).use { stream -> while (true) { if (cancel.get()) throw CancellationException("tile build cancelled"); val n = stream.read(chunk); if (n < 0) break; count += n; if (count > limit) throw TileBuildFailure("source grew"); digest.update(chunk, 0, n) } }
        return digest.digest().joinToString("") { "%02x".format(it.toInt() and 255) }
    }
    private fun rejectProfileAndNonJpeg(file: File): Unit {
        java.io.DataInputStream(FileInputStream(file)).use { stream ->
            if (stream.readUnsignedShort() != 0xffd8) throw TileBuildFailure("JPEG required")
            var read = 2L
            while (read < 16L * 1024 * 1024) {
                if (stream.readUnsignedByte() != 255) throw TileBuildFailure("JPEG metadata")
                var marker = stream.readUnsignedByte(); read += 2
                while (marker == 255) { marker = stream.readUnsignedByte(); if (++read > 16L * 1024 * 1024) throw TileBuildFailure("metadata limit") }
                if (marker == 0xda) return
                if (marker == 0 || marker == 0xd8 || marker == 0xd9 || marker in 0xd0..0xd7) throw TileBuildFailure("JPEG metadata")
                val size = stream.readUnsignedShort() - 2; read += size + 2; if (size < 0 || read > 16L * 1024 * 1024) throw TileBuildFailure("metadata limit")
                val data = ByteArray(size); stream.readFully(data)
                if (marker == 0xe2 && data.size >= 12 && data.copyOfRange(0, 12).contentEquals("ICC_PROFILE\u0000".toByteArray(Charsets.US_ASCII))) throw TileBuildFailure("ICC tile conversion unavailable")
                if (marker in listOf(0xc0, 0xc1, 0xc2) && (data.size != 15 || data[0].toInt() != 8 || data[5].toInt() != 3)) throw TileBuildFailure("JPEG depth/components")
            }
            throw TileBuildFailure("metadata limit")
        }
    }
    internal fun rawRect(x: Int, y: Int, width: Int, height: Int, rawWidth: Int, rawHeight: Int, orientation: Int): Rect {
        fun inverse(dx: Int, dy: Int): Pair<Int, Int> = when (orientation) {
            1 -> dx to dy; 2 -> rawWidth - dx to dy; 3 -> rawWidth - dx to rawHeight - dy; 4 -> dx to rawHeight - dy
            5 -> dy to dx; 6 -> dy to rawHeight - dx; 7 -> rawWidth - dy to rawHeight - dx; 8 -> rawWidth - dy to dx
            else -> throw TileBuildFailure("orientation")
        }
        val a = inverse(x, y); val b = inverse(x + width, y + height)
        return Rect(minOf(a.first, b.first), minOf(a.second, b.second), maxOf(a.first, b.first), maxOf(a.second, b.second)).also {
            if (it.left < 0 || it.top < 0 || it.right > rawWidth || it.bottom > rawHeight || it.isEmpty) throw TileBuildFailure("raw region")
        }
    }
    internal fun orient(bitmap: Bitmap, orientation: Int): Bitmap {
        val matrix = Matrix()
        val coefficients = when (orientation) {
            1 -> return bitmap; 2 -> floatArrayOf(-1f, 0f, 0f, 0f, 1f, 0f, 0f, 0f, 1f)
            3 -> floatArrayOf(-1f, 0f, 0f, 0f, -1f, 0f, 0f, 0f, 1f); 4 -> floatArrayOf(1f, 0f, 0f, 0f, -1f, 0f, 0f, 0f, 1f)
            5 -> floatArrayOf(0f, 1f, 0f, 1f, 0f, 0f, 0f, 0f, 1f); 6 -> floatArrayOf(0f, -1f, 0f, 1f, 0f, 0f, 0f, 0f, 1f)
            7 -> floatArrayOf(0f, -1f, 0f, -1f, 0f, 0f, 0f, 0f, 1f); 8 -> floatArrayOf(0f, 1f, 0f, -1f, 0f, 0f, 0f, 0f, 1f)
            else -> throw TileBuildFailure("orientation")
        }
        matrix.setValues(coefficients); return Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, matrix, false)
    }
    private fun removeWorking(directory: File, files: List<File>): Unit {
        if (directory.canonicalFile != directory.absoluteFile || !directory.name.matches(Regex("R[0-9]{1,2}"))) throw TileBuildFailure("working identity")
        files.filter { it.parentFile == directory }.forEach { if (it.canonicalFile != it.absoluteFile || !it.isFile || !it.delete()) throw TileBuildFailure("working cleanup") }
        if (!directory.delete()) throw TileBuildFailure("working directory cleanup")
    }
    private fun lowerTile(previous: File, x: Int, y: Int, width: Int, height: Int, previousWidth: Int, previousHeight: Int): Bitmap {
        val outWidth = min(256, width - x); val outHeight = min(256, height - y)
        val inputWidth = min(outWidth * 2, previousWidth - x * 2); val inputHeight = min(outHeight * 2, previousHeight - y * 2)
        val joined = Bitmap.createBitmap(inputWidth, inputHeight, Bitmap.Config.ARGB_8888).apply { setHasAlpha(false) }
        try {
            val canvas = Canvas(joined)
            for (dy in 0..1) for (dx in 0..1) {
                if (dx * 256 >= inputWidth || dy * 256 >= inputHeight) continue
                val path = File(previous, "${x * 2 / 256 + dx}_${y * 2 / 256 + dy}.webp")
                val child = BitmapFactory.decodeFile(path.path, options(1)) ?: throw TileBuildFailure("derived tile decode")
                try {
                    admitBitmap(child, 256L * 256 * 4)
                    if (child.width != min(256, inputWidth - dx * 256) || child.height != min(256, inputHeight - dy * 256)) throw TileBuildFailure("derived tile extent")
                    canvas.drawBitmap(child, (dx * 256).toFloat(), (dy * 256).toFloat(), null)
                } finally { child.recycle() }
            }
            val scaled = Bitmap.createScaledBitmap(joined, outWidth, outHeight, true)
            return if (scaled === joined) joined.copy(Bitmap.Config.ARGB_8888, false) ?: throw TileBuildFailure("tile copy") else scaled
        } finally { joined.recycle() }
    }
}

private class CappedOutput(private val output: java.io.OutputStream, private val limit: Long) : java.io.OutputStream() {
    var written = 0L; private set
    override fun write(value: Int): Unit { if (written >= limit) throw TileBuildFailure("encoded tile limit"); output.write(value); written++ }
    override fun write(bytes: ByteArray, offset: Int, count: Int): Unit { if (count < 0 || written > limit - count) throw TileBuildFailure("encoded tile limit"); output.write(bytes, offset, count); written += count }
    override fun flush(): Unit = output.flush()
}
