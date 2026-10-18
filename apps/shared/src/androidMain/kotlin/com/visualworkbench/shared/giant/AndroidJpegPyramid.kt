package com.visualworkbench.shared.giant

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.BitmapRegionDecoder
import android.graphics.Canvas
import android.graphics.ColorSpace
import android.graphics.Matrix
import android.graphics.Rect
import android.os.Build
import com.visualworkbench.shared.SessionResourceCharge
import com.visualworkbench.shared.SessionResources
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.async
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.DataInputStream
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.OutputStream
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.min

private val OWNED_TILE_NAME = Regex("[0-9]{1,4}_[0-9]{1,4}\\.(jpg|webp)")

/** Root's immutable private PreparedOriginal is the sole production caller of
 * fromRootPrepared. This is an internal runner input, not a picker/path API or a
 * producer approval credential. Root must keep its native source owner alive
 * until this operation and its exact output receipt have actually retired. */
internal class JpegTileInput private constructor(
    internal val source: File,
    internal val privateWork: File,
    internal val sha256: String,
    internal val rawWidth: Int,
    internal val rawHeight: Int,
    internal val orientation: Int,
    internal val encodedBytes: Long,
    internal val maxDiskBytes: Long,
    internal val assumeUntaggedSrgb: Boolean,
    internal val rootOperation: Long,
) {
    internal companion object {
        internal fun fromRootPrepared(
            source: File,
            privateWork: File,
            sha256: String,
            rawWidth: Int,
            rawHeight: Int,
            orientation: Int,
            encodedBytes: Long,
            maxDiskBytes: Long,
            assumeUntaggedSrgb: Boolean,
            rootOperation: Long = 0,
        ): JpegTileInput = JpegTileInput(source, privateWork, sha256, rawWidth, rawHeight,
            orientation, encodedBytes, maxDiskBytes, assumeUntaggedSrgb, rootOperation)
    }
}

internal enum class TileBuildPhase { Preview, FullResolution, LowerLevels, Complete }
internal data class TileBuildProgress(internal val phase: TileBuildPhase, internal val completed: Int, internal val total: Int)
internal class TileBuildFailure(internal val reason: String) : Exception(reason)

/** Only the actual settled producer can call the private constructor. Root's
 * trusted Android bridge must consume this typed receipt together with the
 * original native PreparedOriginal, revalidate source/inventory, then close it.
 * There is deliberately no factory accepting an external completed directory.
 * The single producer slot and session envelope survive until successful close. */
internal class AndroidTileBuild private constructor(private val owner: ProducerOwner) {
    private val adoption = Mutex()
    internal val directory: File get() { owner.requireLive(); return owner.directory ?: throw TileBuildFailure("missing output") }
    internal val width: Int get() = owner.width
    internal val height: Int get() = owner.height
    internal val previewWidth: Int get() = owner.previewWidth
    internal val previewHeight: Int get() = owner.previewHeight
    internal val sourceSha256: String get() = owner.input.sha256
    internal val sourceEncodedBytes: Long get() = owner.input.encodedBytes
    internal val rawWidth: Int get() = owner.input.rawWidth
    internal val rawHeight: Int get() = owner.input.rawHeight
    internal val orientation: Int get() = owner.input.orientation

    /** Root's trusted bridge holds this gate through its actually joined native
     * adoption. close cannot remove output while that bridge is reading it. */
    internal suspend fun <T> withRootOutput(consume: suspend (AndroidTileBuild) -> T): T = adoption.withLock {
        currentCoroutineContext().ensureActive()
        owner.requireLive()
        consume(this)
    }

    internal suspend fun close(): Unit = withContext(NonCancellable + Dispatchers.IO) { adoption.withLock { owner.closeNow() } }

    internal companion object {
        internal suspend fun build(
            input: JpegTileInput,
            resources: SessionResources,
            progress: (TileBuildProgress) -> Unit,
        ): AndroidTileBuild {
            currentCoroutineContext().ensureActive()
            requireInput(input)
            val owner = ProducerOwner(input)
            ProducerRuntime.acquire(owner)
            try {
                // The same supplied root-native pool admits the whole operation
                // before hashing buffers, header parsers or any platform codec.
                owner.charge = resources.reserveForeign(AndroidJpegPyramid.SESSION_ENVELOPE_BYTES.toULong())
                currentCoroutineContext().ensureActive()
                val worker = ProducerRuntime.scope.async { owner.produce(progress) }
                try {
                    worker.await()
                    currentCoroutineContext().ensureActive()
                    return AndroidTileBuild(owner)
                } catch (error: Throwable) {
                    owner.cancelled.set(true)
                    // Await cancellation does not stop BitmapRegionDecoder or a
                    // blocking progress callback. Join the actual retained task.
                    withContext(NonCancellable) {
                        try { worker.await() } catch (_: Throwable) { /* Settled worker failure. */ }
                    }
                    throw error
                }
            } catch (error: Throwable) {
                withContext(NonCancellable + Dispatchers.IO) {
                    try { owner.closeNow() } catch (cleanup: Throwable) { error.addSuppressed(cleanup) }
                }
                throw error
            }
        }
    }
}

/** Recovered BitmapRegionDecoder algorithm with mandatory aggregate admission.
 * At most32MiB of raw+oriented band or two16MiB previews coexist; band cropping
 * adds<=256KiB. Lower-level joined/child/scaled storage is<=2MiB, disjoint from
 * preview/bands. The remaining47MiB+ covers conservative decoder/encoder and
 * bounded row/header/hash/output workspace. Every bitmap also passes actual
 * rowBytes/allocationByteCount admission. This80MiB is an admission allowance,
 * not a measured bound on undocumented platform codec/driver RSS. SDK<30 refuses.
 * No full-resolution full-image bitmap is allocated, and no bitmap escapes. */
internal object AndroidJpegPyramid {
    internal const val SESSION_ENVELOPE_BYTES: Long = 80L * 1024 * 1024
    internal const val MAX_BAND_BYTES: Long = 32L * 1024 * 1024
    internal const val MAX_PREVIEW_BYTES: Long = 16L * 1024 * 1024
    internal const val MAX_TILE_BYTES: Long = 2L * 1024 * 1024

    internal suspend fun build(
        input: JpegTileInput,
        resources: SessionResources,
        progress: (TileBuildProgress) -> Unit,
    ): AndroidTileBuild = AndroidTileBuild.build(input, resources, progress)

    internal fun ownsRootOperation(operation: Long): Boolean = ProducerRuntime.ownsRootOperation(operation)

    /** Explicit retry only; an uncertain owner otherwise blocks new work and
     * retains its slot/charge. This never turns cancellation into retirement. */
    internal suspend fun retryQuarantinedCleanup(): Unit = withContext(NonCancellable + Dispatchers.IO) {
        for (owner in ProducerRuntime.quarantined()) owner.closeNow()
    }

    internal fun previewSample(rawWidth: Int, rawHeight: Int): Int {
        if (rawWidth !in 1..1_048_576 || rawHeight !in 1..1_048_576) throw TileBuildFailure("preview source extent")
        val edge = maxOf(rawWidth, rawHeight).toLong()
        var sample = 1
        while ((edge + sample - 1L) / sample > 2048L) sample *= 2
        return sample
    }

    /** Post-allocation admission supplements the pre-admitted session envelope. */
    internal fun admitBitmap(bitmap: Bitmap, limit: Long): Unit {
        admitLayout(bitmap.width, bitmap.height, bitmap.rowBytes.toLong(), bitmap.allocationByteCount.toLong(), limit)
        if (bitmap.isRecycled || bitmap.config != Bitmap.Config.ARGB_8888 || bitmap.colorSpace?.isSrgb != true) {
            throw TileBuildFailure("decoded bitmap format")
        }
        val row = IntArray(min(256, bitmap.width))
        for (y in 0 until bitmap.height) for (x in 0 until bitmap.width step row.size) {
            val count = min(row.size, bitmap.width - x)
            bitmap.getPixels(row, 0, count, x, y, count, 1)
            for (index in 0 until count) if (row[index] ushr 24 != 255) throw TileBuildFailure("nonopaque bitmap")
        }
    }

    internal fun admitLayout(width: Int, height: Int, rowBytes: Long, allocationBytes: Long, limit: Long): Unit {
        if (width <= 0 || height <= 0 || limit <= 0 || rowBytes < width.toLong() * 4 ||
            rowBytes > Long.MAX_VALUE / height || allocationBytes < rowBytes * height || allocationBytes > limit) {
            throw TileBuildFailure("decoded bitmap budget")
        }
    }

    internal fun admitPreview(bitmap: Bitmap): Unit {
        if (maxOf(bitmap.width, bitmap.height) > 2048) throw TileBuildFailure("preview extent")
        admitBitmap(bitmap, MAX_PREVIEW_BYTES)
    }

    internal fun rawRect(x: Int, y: Int, width: Int, height: Int, rawWidth: Int, rawHeight: Int, orientation: Int): Rect {
        val documentWidth = if (orientation >= 5) rawHeight else rawWidth
        val documentHeight = if (orientation >= 5) rawWidth else rawHeight
        if (rawWidth <= 0 || rawHeight <= 0 || x < 0 || y < 0 || width <= 0 || height <= 0 ||
            x.toLong() + width > documentWidth || y.toLong() + height > documentHeight) throw TileBuildFailure("raw region")
        fun inverse(dx: Int, dy: Int): Pair<Int, Int> = when (orientation) {
            1 -> dx to dy
            2 -> rawWidth - dx to dy
            3 -> rawWidth - dx to rawHeight - dy
            4 -> dx to rawHeight - dy
            5 -> dy to dx
            6 -> dy to rawHeight - dx
            7 -> rawWidth - dy to rawHeight - dx
            8 -> rawWidth - dy to dx
            else -> throw TileBuildFailure("orientation")
        }
        val a = inverse(x, y)
        val b = inverse(x + width, y + height)
        return Rect(minOf(a.first, b.first), minOf(a.second, b.second), maxOf(a.first, b.first), maxOf(a.second, b.second)).also {
            if (it.left < 0 || it.top < 0 || it.right > rawWidth || it.bottom > rawHeight || it.isEmpty) throw TileBuildFailure("raw region")
        }
    }

    /** Isolated transform helper; production registers every returned bitmap
     * with its already charged owner before performing admission or other work. */
    internal fun orient(bitmap: Bitmap, orientation: Int): Bitmap {
        val coefficients = when (orientation) {
            1 -> return bitmap
            2 -> floatArrayOf(-1f, 0f, 0f, 0f, 1f, 0f, 0f, 0f, 1f)
            3 -> floatArrayOf(-1f, 0f, 0f, 0f, -1f, 0f, 0f, 0f, 1f)
            4 -> floatArrayOf(1f, 0f, 0f, 0f, -1f, 0f, 0f, 0f, 1f)
            5 -> floatArrayOf(0f, 1f, 0f, 1f, 0f, 0f, 0f, 0f, 1f)
            6 -> floatArrayOf(0f, -1f, 0f, 1f, 0f, 0f, 0f, 0f, 1f)
            7 -> floatArrayOf(0f, -1f, 0f, -1f, 0f, 0f, 0f, 0f, 1f)
            8 -> floatArrayOf(0f, 1f, 0f, -1f, 0f, 0f, 0f, 0f, 1f)
            else -> throw TileBuildFailure("orientation")
        }
        val matrix = Matrix().apply { setValues(coefficients) }
        return Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, matrix, false)
    }
}

private object ProducerRuntime {
    private val slots = Semaphore(1)
    private val quarantine = mutableSetOf<ProducerOwner>()
    // Also retain successful undelivered results: losing a caller reference must
    // not let a native charge cleaner bypass this operation's explicit close.
    private var active: ProducerOwner? = null
    private val dispatcher by lazy {
        Executors.newSingleThreadExecutor { task -> Thread(task, "vw-jpeg-pyramid").apply { isDaemon = true } }.asCoroutineDispatcher()
    }
    val scope: CoroutineScope by lazy { CoroutineScope(SupervisorJob() + dispatcher) }
    @Synchronized fun acquire(owner: ProducerOwner): Unit {
        if (active != null || quarantine.isNotEmpty() || !slots.tryAcquire()) throw TileBuildFailure("busy")
        active = owner
    }
    @Synchronized fun retain(owner: ProducerOwner): Unit { quarantine.add(owner) }
    @Synchronized fun release(owner: ProducerOwner): Unit {
        if (active !== owner) throw TileBuildFailure("producer slot identity")
        quarantine.remove(owner); active = null; slots.release()
    }
    @Synchronized fun quarantined(): List<ProducerOwner> = quarantine.toList()
    @Synchronized fun ownsRootOperation(operation: Long): Boolean = operation > 0 &&
        (active?.input?.rootOperation == operation || quarantine.any { it.input.rootOperation == operation })
}

private fun requireInput(input: JpegTileInput): Unit {
    if (Build.VERSION.SDK_INT < 30) throw TileBuildFailure("SDK30 required")
    if (!input.assumeUntaggedSrgb) throw TileBuildFailure("untagged sRGB consent required")
    if (input.rawWidth !in 1..1_048_576 || input.rawHeight !in 1..1_048_576 ||
        input.rawWidth.toLong() * input.rawHeight > 1_000_000_000L || input.orientation !in 1..8 ||
        input.encodedBytes !in 33..1024L * 1024 * 1024 || input.maxDiskBytes !in 1..16L * 1024 * 1024 * 1024 ||
        !input.sha256.matches(Regex("[0-9a-f]{64}"))) throw TileBuildFailure("input limit")
    if (input.rawWidth.toLong() * input.rawHeight * 8L + 64L * 1024 * 1024 > input.maxDiskBytes) throw TileBuildFailure("disk budget")
    val width = if (input.orientation >= 5) input.rawHeight else input.rawWidth
    val height = if (input.orientation >= 5) input.rawWidth else input.rawHeight
    if (width.toLong() * min(256, height) * 8L > AndroidJpegPyramid.MAX_BAND_BYTES) throw TileBuildFailure("band budget")
}

private data class SourceStamp(val key: Any, val size: Long, val modified: Long)
private data class OwnedOutput(val parent: File, val name: String) {
    fun file(): File = File(parent, name)
}
private fun sourceStamp(input: JpegTileInput): SourceStamp {
    val path = input.source.toPath()
    val attributes = Files.readAttributes(path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
    val key = attributes.fileKey() ?: throw TileBuildFailure("source identity")
    if (!attributes.isRegularFile || attributes.isSymbolicLink || input.source.canonicalFile != input.source.absoluteFile ||
        attributes.size() != input.encodedBytes) throw TileBuildFailure("source binding")
    return SourceStamp(key, attributes.size(), attributes.lastModifiedTime().toMillis())
}

private class ProducerOwner(val input: JpegTileInput) {
    val cancelled: AtomicBoolean = AtomicBoolean(false)
    var charge: SessionResourceCharge? = null
    var directory: File? = null
        private set
    val width: Int = if (input.orientation >= 5) input.rawHeight else input.rawWidth
    val height: Int = if (input.orientation >= 5) input.rawWidth else input.rawHeight
    var previewWidth: Int = 0
        private set
    var previewHeight: Int = 0
        private set
    private val marker = UUID.randomUUID().toString()
    // Retain compact relative names and shared level owners, never thousands of
    // copies of the full app-private path. <=65536 small records fit within the
    // envelope's8MiB metadata allowance even at the admitted1Gpixel ceiling.
    private val files = ArrayList<OwnedOutput>()
    private val directories = ArrayList<File>()
    // Capacity is established before any bitmap/decoder allocation, so recording
    // a returned platform owner does not require a growing registration array.
    private val bitmaps = ArrayList<Bitmap>(8)
    private val decoders = ArrayList<BitmapRegionDecoder>(1)
    private val canvases = ArrayList<Canvas>(1)
    private var disk = 0L
    private var filesRetired = false
    private var closed = false
    private var complete = false
    private var quarantined = false

    @Synchronized fun requireLive(): Unit {
        if (!complete || closed || filesRetired || quarantined) throw TileBuildFailure("build retired")
    }

    private fun check(): Unit { if (cancelled.get()) throw CancellationException("tile build cancelled") }
    private fun keep(bitmap: Bitmap?, limit: Long): Bitmap {
        val actual = bitmap ?: throw TileBuildFailure("bitmap unavailable")
        if (bitmaps.none { it === actual }) bitmaps.add(actual)
        AndroidJpegPyramid.admitBitmap(actual, limit)
        val live = bitmaps.sumOf { it.allocationByteCount.toLong() }
        if (live > AndroidJpegPyramid.MAX_BAND_BYTES + 256L * 256 * 4) throw TileBuildFailure("live bitmap budget")
        check()
        return actual
    }
    private fun retire(bitmap: Bitmap): Unit {
        if (!bitmap.isRecycled) bitmap.recycle()
        if (!bitmap.isRecycled) throw TileBuildFailure("bitmap retirement")
        bitmaps.removeAll { it === bitmap }
    }
    private fun retireDecoder(decoder: BitmapRegionDecoder): Unit {
        if (!decoder.isRecycled) decoder.recycle()
        if (!decoder.isRecycled) throw TileBuildFailure("decoder retirement")
        decoders.removeAll { it === decoder }
    }
    private fun retireCanvas(canvas: Canvas): Unit {
        canvas.setBitmap(null)
        canvases.removeAll { it === canvas }
    }
    private fun level(name: String): File {
        val root = directory ?: throw TileBuildFailure("missing output")
        val path = File(root, name)
        if (directories.size >= 64) throw TileBuildFailure("level metadata limit")
        if (!path.mkdir()) throw TileBuildFailure("staging level")
        directories.add(path)
        return path
    }
    private fun save(bitmap: Bitmap, file: File, lossless: Boolean): Unit {
        check()
        if (files.size >= 65536) throw TileBuildFailure("file metadata limit")
        val root = directory ?: throw TileBuildFailure("missing output")
        val parent = if (file.parentFile == root) root else directories.firstOrNull { it == file.parentFile }
            ?: throw TileBuildFailure("output parent")
        val name = file.name
        if (name != "preview.jpg" && !name.matches(OWNED_TILE_NAME)) throw TileBuildFailure("output name")
        if (!file.createNewFile()) throw TileBuildFailure("tile exists")
        files.add(OwnedOutput(parent, name))
        FileOutputStream(file).use { stream ->
            val limit = if (file.name == "preview.jpg") 16L * 1024 * 1024 else AndroidJpegPyramid.MAX_TILE_BYTES
            val output = CappedOutput(stream, min(limit, input.maxDiskBytes - disk))
            val format = if (lossless) Bitmap.CompressFormat.WEBP_LOSSLESS else Bitmap.CompressFormat.JPEG
            if (!bitmap.compress(format, if (lossless) 100 else 90, output)) throw TileBuildFailure("tile encode")
            output.flush(); stream.fd.sync(); disk += output.written
        }
        check()
    }

    fun produce(progress: (TileBuildProgress) -> Unit): Unit {
        check()
        if (charge == null) throw TileBuildFailure("session reservation required")
        val work = input.privateWork
        if (!work.isDirectory || work.canonicalFile != work.absoluteFile ||
            work.parentFile != input.source.absoluteFile.parentFile) throw TileBuildFailure("private work")
        val stamp = sourceStamp(input)
        FileInputStream(input.source).use { source ->
            source.channel.lock(0L, Long.MAX_VALUE, true).use {
                if (hash(source, input.encodedBytes, cancelled) != input.sha256) throw TileBuildFailure("source hash")
                source.channel.position(0)
                admitJpegHeader(source, input)
                source.channel.position(0)
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFileDescriptor(source.fd, null, bounds)
                if (bounds.outWidth != input.rawWidth || bounds.outHeight != input.rawHeight || bounds.outMimeType != "image/jpeg" ||
                    bounds.outColorSpace?.isSrgb != true) throw TileBuildFailure("source metadata")
                checkSource(stamp)
                check()
                val root = File(work, "vw-jpeg-$marker")
                if (!root.mkdir()) throw TileBuildFailure("staging")
                directory = root
                FileOutputStream(File(root, ".owner")).use { stream -> stream.write(marker.toByteArray(Charsets.US_ASCII)); stream.fd.sync() }
                disk = marker.length.toLong()
                source.channel.position(0)
                @Suppress("DEPRECATION")
                val decoder = BitmapRegionDecoder.newInstance(source.fd, false) ?: throw TileBuildFailure("region decoder")
                decoders.add(decoder)
                try {
                    val sample = AndroidJpegPyramid.previewSample(input.rawWidth, input.rawHeight)
                    val preview = keep(decoder.decodeRegion(Rect(0, 0, input.rawWidth, input.rawHeight), options(sample)), AndroidJpegPyramid.MAX_PREVIEW_BYTES)
                    try {
                        AndroidJpegPyramid.admitPreview(preview)
                        val oriented = keep(AndroidJpegPyramid.orient(preview, input.orientation), AndroidJpegPyramid.MAX_PREVIEW_BYTES)
                        try {
                            AndroidJpegPyramid.admitPreview(oriented)
                            previewWidth = oriented.width; previewHeight = oriented.height
                            save(oriented, File(root, "preview.jpg"), false)
                        } finally { if (oriented !== preview) retire(oriented) }
                    } finally { retire(preview) }
                    progress(TileBuildProgress(TileBuildPhase.Preview, 0, 1))
                    checkSource(stamp)
                    val base = level("L0")
                    val total = ((width + 255) / 256) * ((height + 255) / 256)
                    var done = 0
                    for (y in 0 until height step 256) {
                        check()
                        val bandHeight = min(256, height - y)
                        val rect = AndroidJpegPyramid.rawRect(0, y, width, bandHeight, input.rawWidth, input.rawHeight, input.orientation)
                        val raw = keep(decoder.decodeRegion(rect, options(1)), AndroidJpegPyramid.MAX_BAND_BYTES / 2)
                        try {
                            if (raw.width != rect.width() || raw.height != rect.height()) throw TileBuildFailure("band extent")
                            val band = keep(AndroidJpegPyramid.orient(raw, input.orientation), AndroidJpegPyramid.MAX_BAND_BYTES / 2)
                            try {
                                if (band.width != width || band.height != bandHeight) throw TileBuildFailure("oriented band extent")
                                for (x in 0 until width step 256) {
                                    check()
                                    val tile = keep(Bitmap.createBitmap(band, x, 0, min(256, width - x), bandHeight), 256L * 256 * 4)
                                    try { save(tile, File(base, "${x / 256}_${y / 256}.webp"), true) }
                                    finally { if (tile !== band) retire(tile) }
                                    progress(TileBuildProgress(TileBuildPhase.FullResolution, ++done, total))
                                }
                            } finally { if (band !== raw) retire(band) }
                        } finally { retire(raw) }
                        checkSource(stamp)
                    }
                } finally {
                    retireDecoder(decoder)
                }
                var previousWidth = width; var previousHeight = height; var number = 1
                while (previousWidth != 1 || previousHeight != 1) {
                    check()
                    val nextWidth = (previousWidth + 1) / 2; val nextHeight = (previousHeight + 1) / 2
                    val target = level("L$number")
                    val lossless = level("R$number")
                    val previous = File(root, if (number == 1) "L0" else "R${number - 1}")
                    val total = ((nextWidth + 255) / 256) * ((nextHeight + 255) / 256)
                    var done = 0
                    for (y in 0 until nextHeight step 256) for (x in 0 until nextWidth step 256) {
                        check()
                        val tile = lowerTile(previous, x, y, nextWidth, nextHeight, previousWidth, previousHeight)
                        try {
                            save(tile, File(lossless, "${x / 256}_${y / 256}.webp"), true)
                            save(tile, File(target, "${x / 256}_${y / 256}.jpg"), false)
                        } finally { retire(tile) }
                        progress(TileBuildProgress(TileBuildPhase.LowerLevels, ++done, total))
                    }
                    if (number > 1) removeWorking(previous)
                    previousWidth = nextWidth; previousHeight = nextHeight; number++
                }
                if (number > 1) removeWorking(File(root, "R${number - 1}"))
                checkSource(stamp)
                if (hash(source, input.encodedBytes, cancelled) != input.sha256) throw TileBuildFailure("source changed")
                checkSource(stamp)
                check()
                if (bitmaps.isNotEmpty()) throw TileBuildFailure("bitmap retirement")
                progress(TileBuildProgress(TileBuildPhase.Complete, 1, 1))
            }
        }
        complete = true
    }

    private fun checkSource(expected: SourceStamp): Unit {
        check()
        if (sourceStamp(input) != expected) throw TileBuildFailure("source changed")
    }

    private fun lowerTile(previous: File, x: Int, y: Int, width: Int, height: Int, previousWidth: Int, previousHeight: Int): Bitmap {
        val outWidth = min(256, width - x); val outHeight = min(256, height - y)
        val inputWidth = min(outWidth * 2, previousWidth - x * 2); val inputHeight = min(outHeight * 2, previousHeight - y * 2)
        val joined = Bitmap.createBitmap(inputWidth, inputHeight, Bitmap.Config.ARGB_8888)
        // Newly created storage is ours; initialize opaque black before admission.
        bitmaps.add(joined)
        joined.eraseColor(android.graphics.Color.BLACK); joined.setHasAlpha(false)
        keep(joined, 512L * 512 * 4)
        try {
            val canvas = Canvas(joined)
            canvases.add(canvas)
            try {
                for (dy in 0..1) for (dx in 0..1) {
                    check()
                    if (dx * 256 >= inputWidth || dy * 256 >= inputHeight) continue
                    val path = File(previous, "${x * 2 / 256 + dx}_${y * 2 / 256 + dy}.webp")
                    val child = keep(BitmapFactory.decodeFile(path.path, options(1)), 256L * 256 * 4)
                    try {
                        if (child.width != min(256, inputWidth - dx * 256) || child.height != min(256, inputHeight - dy * 256)) throw TileBuildFailure("derived tile extent")
                        canvas.drawBitmap(child, (dx * 256).toFloat(), (dy * 256).toFloat(), null)
                    } finally { retire(child) }
                }
            } finally { retireCanvas(canvas) }
            val scaled = keep(Bitmap.createScaledBitmap(joined, outWidth, outHeight, true), 256L * 256 * 4)
            return if (scaled === joined) keep(joined.copy(Bitmap.Config.ARGB_8888, false), 256L * 256 * 4) else scaled
        } finally { retire(joined) }
    }

    private fun removeWorking(path: File): Unit {
        if (path.canonicalFile != path.absoluteFile || !path.name.matches(Regex("R[0-9]{1,2}"))) throw TileBuildFailure("working identity")
        for (entry in files.filter { it.parent == path }) {
            val file = entry.file()
            if (!file.exists()) continue
            val bytes = file.length()
            if (file.canonicalFile != file.absoluteFile || !file.isFile || !file.delete()) throw TileBuildFailure("working cleanup")
            disk -= bytes
        }
        if (!path.delete()) throw TileBuildFailure("working directory cleanup")
    }

    @Synchronized fun closeNow(): Unit {
        if (closed) return
        try {
            // A failed recycle retains the bitmap and the entire envelope.
            for (canvas in canvases.toList()) retireCanvas(canvas)
            for (bitmap in bitmaps.toList()) retire(bitmap)
            for (decoder in decoders.toList()) retireDecoder(decoder)
            if (!filesRetired) {
                cleanupFiles()
                filesRetired = true
            }
            charge?.close()
            charge = null
            closed = true
            ProducerRuntime.release(this)
        } catch (error: Throwable) {
            quarantined = true
            ProducerRuntime.retain(this)
            throw error
        }
        // The source pin survives every failed close. Notify only after files,
        // native charge and slot have all actually retired, including on retry.
        AndroidGiantRuntime.ownerRetired(input.rootOperation)
    }

    private fun cleanupFiles(): Unit {
        val root = directory ?: return
        val markerFile = File(root, ".owner")
        if (!root.isDirectory || root.canonicalFile != root.absoluteFile || markerFile.canonicalFile.parentFile != root ||
            !markerFile.isFile || markerFile.length() != marker.length.toLong()) throw TileBuildFailure("cleanup identity")
        val markerBytes = ByteArray(marker.length)
        DataInputStream(FileInputStream(markerFile)).use { stream ->
            stream.readFully(markerBytes)
            if (stream.read() != -1 || !markerBytes.contentEquals(marker.toByteArray(Charsets.US_ASCII))) throw TileBuildFailure("cleanup identity")
        }
        for (entry in files.asReversed()) {
            val file = entry.file()
            if (!file.exists()) continue
            if (file.canonicalFile != file.absoluteFile || !file.isFile || file.parentFile !in directories && file.parentFile != root ||
                !file.delete()) throw TileBuildFailure("cleanup file")
        }
        for (path in directories.asReversed()) {
            if (!path.exists()) continue
            if (path.canonicalFile != path.absoluteFile || path.parentFile != root || !path.isDirectory || !path.delete()) throw TileBuildFailure("cleanup directory")
        }
        Files.newDirectoryStream(root.toPath()).use { entries ->
            val iterator = entries.iterator()
            if (!iterator.hasNext() || iterator.next() != markerFile.toPath() || iterator.hasNext()) throw TileBuildFailure("cleanup foreign entry")
        }
        if (!markerFile.delete() || !root.delete()) throw TileBuildFailure("cleanup retained")
    }
}

private fun options(sample: Int): BitmapFactory.Options = BitmapFactory.Options().apply {
    inSampleSize = sample; inPreferredConfig = Bitmap.Config.ARGB_8888; inScaled = false
    inPreferredColorSpace = ColorSpace.get(ColorSpace.Named.SRGB)
}

private fun hash(source: FileInputStream, limit: Long, cancel: AtomicBoolean): String {
    source.channel.position(0)
    val digest = MessageDigest.getInstance("SHA-256")
    val buffer = ByteArray(65536)
    var count = 0L
    while (true) {
        if (cancel.get()) throw CancellationException("tile build cancelled")
        val read = source.read(buffer)
        if (read < 0) break
        count += read
        if (count > limit) throw TileBuildFailure("source grew")
        digest.update(buffer, 0, read)
    }
    if (count != limit) throw TileBuildFailure("source length")
    return digest.digest().joinToString("") { "%02x".format(it.toInt() and 255) }
}

/** Nonallocating-size-capped source header census. No ICC/XMP/MPF/JUMBF/HDR,
 * CMYK/depth conversion or orientation override is admitted. EXIF admission is
 * intentionally limited to one orientation-only IFD, with no nested/thumbnail
 * metadata. Other valid JPEG metadata remains unsupported rather than stripped. */
private fun admitJpegHeader(source: FileInputStream, input: JpegTileInput): Unit {
    val stream = DataInputStream(source)
    if (stream.readUnsignedShort() != 0xffd8) throw TileBuildFailure("JPEG required")
    var read = 2L
    var frame = false
    var exif = false
    var orientation = 1
    while (read < 16L * 1024 * 1024) {
        if (stream.readUnsignedByte() != 255) throw TileBuildFailure("JPEG metadata")
        var marker = stream.readUnsignedByte(); read += 2
        while (marker == 255) { marker = stream.readUnsignedByte(); if (++read > 16L * 1024 * 1024) throw TileBuildFailure("metadata limit") }
        val size = stream.readUnsignedShort() - 2
        read += size + 2
        if (size < 0 || read > 16L * 1024 * 1024) throw TileBuildFailure("metadata limit")
        val data = ByteArray(size)
        stream.readFully(data)
        when (marker) {
            0xda -> {
                if (!frame || orientation != input.orientation) throw TileBuildFailure("source orientation")
                return
            }
            0xe0 -> if (data.size !in 14..32 || !data.copyOfRange(0, 5).contentEquals("JFIF\u0000".toByteArray(Charsets.US_ASCII))) throw TileBuildFailure("JPEG metadata")
            0xe1 -> {
                if (exif) throw TileBuildFailure("JPEG metadata")
                orientation = exifOrientation(data); exif = true
            }
            0xdb, 0xc4 -> Unit
            0xdd -> if (size != 2) throw TileBuildFailure("JPEG metadata")
            0xc0, 0xc1, 0xc2 -> {
                if (frame || data.size != 15 || data[0].toInt() != 8 || data[5].toInt() != 3 ||
                    ((data[1].toInt() and 255) shl 8 or (data[2].toInt() and 255)) != input.rawHeight ||
                    ((data[3].toInt() and 255) shl 8 or (data[4].toInt() and 255)) != input.rawWidth) throw TileBuildFailure("JPEG depth/components/extent")
                frame = true
            }
            else -> throw TileBuildFailure("tagged or unsupported JPEG")
        }
    }
    throw TileBuildFailure("metadata limit")
}

private fun exifOrientation(data: ByteArray): Int {
    if (data.size != 32 || !data.copyOfRange(0, 6).contentEquals("Exif\u0000\u0000".toByteArray(Charsets.US_ASCII))) throw TileBuildFailure("unsupported EXIF")
    val little = data[6] == 'I'.code.toByte() && data[7] == 'I'.code.toByte()
    if (!little && !(data[6] == 'M'.code.toByte() && data[7] == 'M'.code.toByte())) throw TileBuildFailure("unsupported EXIF")
    fun u16(at: Int): Int = if (little) (data[at].toInt() and 255) or ((data[at + 1].toInt() and 255) shl 8)
        else ((data[at].toInt() and 255) shl 8) or (data[at + 1].toInt() and 255)
    fun u32(at: Int): Long = if (little) u16(at).toLong() or (u16(at + 2).toLong() shl 16)
        else (u16(at).toLong() shl 16) or u16(at + 2).toLong()
    if (u16(8) != 42 || u32(10) != 8L || u16(14) != 1 || u16(16) != 0x0112 ||
        u16(18) != 3 || u32(20) != 1L || u16(26) != 0 || u32(28) != 0L) throw TileBuildFailure("unsupported EXIF")
    return u16(24).also { if (it !in 1..8) throw TileBuildFailure("source orientation") }
}

private class CappedOutput(private val output: OutputStream, private val limit: Long) : OutputStream() {
    var written: Long = 0
        private set
    override fun write(value: Int): Unit {
        if (written >= limit) throw TileBuildFailure("encoded tile limit")
        output.write(value); written++
    }
    override fun write(bytes: ByteArray, offset: Int, count: Int): Unit {
        if (offset < 0 || count < 0 || offset > bytes.size - count || written > limit - count) throw TileBuildFailure("encoded tile limit")
        output.write(bytes, offset, count); written += count
    }
    override fun flush(): Unit = output.flush()
}
