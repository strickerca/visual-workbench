package com.visualworkbench.android.editor

import android.content.Context
import android.net.Uri
import com.visualworkbench.shared.*
import com.visualworkbench.shared.giant.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.File
import java.io.FileOutputStream

internal const val GIANT_PHONE_ORIGINAL_BYTES = 1024L * 1024L * 1024L
internal const val GIANT_PHONE_WORK_BYTES = 16L * 1024L * 1024L * 1024L

/** A cancelled return cannot prove that native no-clobber publication did not
 * finish. The complete project remains discoverable; no rollback deletes it. */
internal class GiantAndroidPublicationCancelled(val projectPath: String, cause: CancellationException) :
    CancellationException("Giant project delivery was canceled; a completed project may remain at $projectPath") {
    init { initCause(cause) }
}

internal interface GiantBuiltCache {
    val directory: String
    suspend fun close()
}
private suspend fun buildAndroidCache(input: JpegTileInput, work: File, progress: (TileBuildProgress) -> Unit): GiantBuiltCache {
    val built = AndroidJpegPyramid.build(input, work, progress)
    return object : GiantBuiltCache {
        override val directory = built.directory.absolutePath
        override suspend fun close() { built.close() }
    }
}

/** Conservative complete intake disk admission includes four original copies
 * and two producer/cache envelopes. It is not a measured filesystem/RSS claim. */
internal fun giantPhoneWorkingBytes(info: GiantInputInfo): Long {
    require(info.format == "jpeg" && info.orientation.toInt() in 1..8 && info.encodedBytes in 1uL..GIANT_PHONE_ORIGINAL_BYTES.toULong()) {
        "This phone's giant import supports untagged opaque 8-bit JPEG originals up to 1 GiB."
    }
    val pixels = info.width.toULong() * info.height.toULong()
    require(pixels in 50_000_001uL..1_000_000_000uL && info.width > 0u && info.height > 0u)
    val oriented = if (info.orientation.toInt() >= 5) info.rawHeight to info.rawWidth else info.rawWidth to info.rawHeight
    require(oriented == (info.width to info.height)) { "Giant source orientation differs from its metadata" }
    require(info.width.toULong() * minOf(info.height, 256u).toULong() * 8uL <= 32uL * 1024uL * 1024uL) {
        "This image's full-resolution row band exceeds the phone decoder allowance. Use the computer to build its tile cache."
    }
    val peak = info.encodedBytes * 4uL + pixels * 24uL + 128uL * 1024uL * 1024uL
    require(peak <= GIANT_PHONE_WORK_BYTES.toULong()) { "This giant image exceeds the private working-storage allowance." }
    return peak.toLong()
}

/** Only explicit giant intake calls this adapter. Ordinary photo/share import
 * retains its 50 MP and encoded-size refusals. A single owner serializes build,
 * publication and uncertain cleanup; no provider FD reaches the codec. */
internal class GiantAndroidInputs(
    context: Context,
    private val prepare: suspend (GiantInputOptions) -> GiantPreparedSource = ::prepareGiantImage,
    private val build: suspend (JpegTileInput, File, (TileBuildProgress) -> Unit) -> GiantBuiltCache = ::buildAndroidCache,
    private val availableBytes: (File) -> Long = { it.usableSpace },
) {
    private val context = context.applicationContext
    private val provider = ProviderIo(this.context.contentResolver)
    private val gate = Mutex()
    private var retained: Resources? = null

    private class Resources(val work: RasterTransfers.Workspace) {
        var source: GiantPreparedSource? = null
        var built: GiantBuiltCache? = null
        var undelivered: WorkbenchProject? = null
        val open: Boolean get() = undelivered != null || built != null || source != null || work.isOpen
        suspend fun closeInputs() = withContext(NonCancellable) {
            built?.close(); built = null
            source?.close(); source = null
            if (work.isOpen) withContext(Dispatchers.IO) { work.close() }
        }
        suspend fun close() = withContext(NonCancellable) {
            undelivered?.close(); undelivered = null
            closeInputs()
        }
    }

    suspend fun settle() = gate.withLock {
        retained?.close(); retained = null
    }

    suspend fun importJpeg(uri: Uri, options: (GiantInputInfo, String) -> GiantProjectOptions,
        stage: (String) -> Unit): WorkbenchProject = gate.withLock {
        currentCoroutineContext().ensureActive()
        require(uri.scheme == "content") { "Choose a JPEG from an Android document provider." }
        retained?.close(); retained = null
        val resources = Resources(RasterTransfers.Workspace(context.cacheDir))
        var publishing: String? = null
        try {
            stage("Copying the unchanged giant JPEG original…")
            val title = withContext(Dispatchers.IO) {
                resources.work.create()
                copyGiantOriginal(provider, uri, resources.work.file("original.bin"), GIANT_PHONE_ORIGINAL_BYTES)
                provider.title(uri)
            }
            resources.source = prepare(GiantInputOptions(resources.work.file("original.bin").absolutePath,
                resources.work.root.absolutePath, GIANT_PHONE_ORIGINAL_BYTES.toULong()))
            val source = checkNotNull(resources.source)
            val info = source.describe()
            val required = giantPhoneWorkingBytes(info)
            withContext(Dispatchers.IO) {
                require(availableBytes(resources.work.root) >= required) { "Insufficient private storage for complete full-resolution tiling and publication." }
            }
            build(resources, info, stage)
            val destination = options(info, title)
            require(destination.diskCacheBytes <= GIANT_PHONE_WORK_BYTES.toULong())
            stage("Publishing the original and complete tile cache…")
            publishing = destination.path
            resources.undelivered = source.publish(destination, checkNotNull(resources.built).directory)
            // Settle all disposable producer/source files before returning the
            // project. The complete published project is never cleanup scratch.
            resources.closeInputs()
            currentCoroutineContext().ensureActive()
            val result = checkNotNull(resources.undelivered)
            resources.undelivered = null
            result
        } catch (cancel: CancellationException) {
            if (publishing != null) throw GiantAndroidPublicationCancelled(checkNotNull(publishing), cancel)
            throw cancel
        } finally {
            // Retain the exact owner if cleanup is uncertain. A later retry or
            // ViewModel teardown settles it; never recursively delete strangers.
            if (resources.open) {
                retained = resources
                resources.close()
                retained = null
            }
        }
    }

    suspend fun rebuild(project: WorkbenchProject, documentId: String, stage: (String) -> Unit) = gate.withLock {
        currentCoroutineContext().ensureActive()
        retained?.close(); retained = null
        val resources = Resources(RasterTransfers.Workspace(context.cacheDir))
        try {
            withContext(Dispatchers.IO) { resources.work.create() }
            stage("Reading the preserved full-resolution original…")
            resources.source = project.prepareGiantRebuild(documentId, resources.work.root.absolutePath)
            val source = checkNotNull(resources.source)
            val info = source.describe()
            val required = giantPhoneWorkingBytes(info)
            withContext(Dispatchers.IO) { require(availableBytes(resources.work.root) >= required) { "Insufficient private storage to rebuild the complete cache." } }
            build(resources, info, stage)
            stage("Installing the complete original-bound tile cache…")
            source.rebuild(project, documentId, checkNotNull(resources.built).directory, GIANT_PHONE_WORK_BYTES.toULong())
        } finally {
            retained = resources; resources.close(); retained = null
        }
    }

    private suspend fun build(resources: Resources, info: GiantInputInfo, stage: (String) -> Unit) = coroutineScope {
        val progress = Channel<TileBuildProgress>(Channel.CONFLATED)
        val reporter = launch {
            for (value in progress) stage(when (value.phase) {
                TileBuildPhase.Preview -> "Preview decoded; building the full-resolution cache…"
                TileBuildPhase.FullResolution -> "Building full-resolution tiles ${value.completed}/${value.total}…"
                TileBuildPhase.LowerLevels -> "Building zoom-level tiles ${value.completed}/${value.total}…"
                TileBuildPhase.Complete -> "Verifying the complete tile cache…"
            })
        }
        try {
            resources.built = build(JpegTileInput(info.sourcePath, info.sha256, info.rawWidth.toInt(), info.rawHeight.toInt(),
                info.orientation.toInt(), info.encodedBytes.toLong(), maxDiskBytes = GIANT_PHONE_WORK_BYTES), resources.work.root) { progress.trySend(it) }
        } finally { progress.close(); withContext(NonCancellable) { reporter.cancelAndJoin() } }
    }
}

internal suspend fun copyGiantOriginal(provider: ProviderIo, uri: Uri, destination: File, limit: Long) {
    require(uri.scheme == "content" && limit in 1..GIANT_PHONE_ORIGINAL_BYTES)
    check(destination.createNewFile())
    provider.open(uri, "r").use { input ->
        provider.nonBlocking(input)
        FileOutputStream(destination).use { output ->
            val buffer = ByteArray(64 * 1024)
            var total = 0L
            while (true) {
                currentCoroutineContext().ensureActive()
                val count = provider.read(input.fileDescriptor, buffer)
                if (count == 0) break
                require(count > 0 && total <= limit - count) { "The encoded giant original exceeds the phone's explicit input limit." }
                output.write(buffer, 0, count); total += count
            }
            require(total > 0) { "The selected original is empty" }
            output.fd.sync()
        }
    }
}
