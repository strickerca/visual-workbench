package com.visualworkbench.android.editor

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.File
import java.io.FileOutputStream
import java.security.MessageDigest
import kotlin.math.floor

internal const val GIANT_PNG_FILE_LIMIT = 4L * 1024L * 1024L * 1024L
internal const val GIANT_PNG_SHARE_LIMIT = 128L * 1024L * 1024L

/** Main-owned, one retained private PNG. A check actually renders and verifies
 * the exact snapshot; publication never rerenders a different revision. The
 * renderer callback must join its real bitmap/surface retirement before native
 * encoding. Native completion is settled before any scratch cleanup. */
internal class GiantAndroidExports(
    context: Context,
    private val render: suspend (GiantPngOptions) -> GiantPngReceipt,
    private val handoffFactory: (Context) -> MediaHandoff = { MediaHandoff(it) },
    private val availableBytes: (File) -> Long = { it.usableSpace },
) {
    private val context = context.applicationContext
    private val provider = ProviderIo(this.context.contentResolver)
    private val gate = Mutex()
    private data class Completed(val binding: WorkflowBinding, val options: ExportOptions,
        val receipt: GiantPngReceipt, val width: UInt, val height: UInt, val privateSha256: ByteArray)
    private var work: RasterTransfers.Workspace? = null
    private var completed: Completed? = null
    private var pendingShare: MediaHandoff.Lease? = null
    private var shareOwner: MediaHandoff? = null

    suspend fun prepare(binding: WorkflowBinding, width: UInt, height: UInt, options: ExportOptions,
        stage: (String) -> Unit): FilePreflight = gate.withLock {
        currentCoroutineContext().ensureActive()
        val dimensions = giantPngDimensions(binding, width, height, options)
        clearLocked()
        val next = RasterTransfers.Workspace(context.cacheDir)
        work = next // Retain before a cancellable filesystem dispatcher handoff.
        try {
            stage("Rendering the original-resolution PNG into private storage…")
            val outputPath = withContext(Dispatchers.IO) {
                next.create()
                require(availableBytes(next.root) >= GIANT_PNG_FILE_LIMIT + 64L * 1024L * 1024L) {
                    "Giant PNG export needs at least 4 GiB plus temporary overhead available in private storage."
                }
                next.file("output.png").absolutePath
            }
            val receipt = render(GiantPngOptions(binding, outputPath,
                options.marked, options.region, options.assumeUntaggedSrgb, options.memoryBudgetBytes,
                GIANT_PNG_FILE_LIMIT.toULong()))
            verifyGiantPngReceipt(binding, options, receipt)
            val digest = withContext(Dispatchers.IO) { verifyFile(next, receipt) }
            currentCoroutineContext().ensureActive()
            completed = Completed(binding, options, receipt, dimensions.first, dimensions.second, digest)
            FilePreflight(dimensions.first, dimensions.second, 8u, 8u, receipt.estimatedPeakBytes,
                buffered = false, requiresRenderValidation = false, revision = receipt.revision)
        } catch (error: Throwable) {
            withContext(NonCancellable) { clearLocked() }
            throw error
        }
    }

    suspend fun save(uri: Uri, binding: WorkflowBinding, options: ExportOptions,
        stage: (String) -> Unit): FileExportResult {
        require(uri.scheme == "content") { "Choose an Android document provider destination." }
        var published = false
        try {
            return gate.withLock {
            val value = checked(binding, options) // No provider access before exact binding admission.
            val owned = checkNotNull(work)
            stage("Saving the checked original-resolution PNG…")
            withContext(Dispatchers.IO) {
                verifyFile(owned, value.receipt, value.privateSha256)
                provider.open(uri, "wt").use { destination ->
                    provider.nonBlocking(destination)
                    owned.file("output.png").inputStream().use { input ->
                        val buffer = ByteArray(64 * 1024); var count = 0uL
                        while (true) {
                            ensureActive()
                            val size = input.read(buffer)
                            if (size < 0) break
                            if (size == 0) continue
                            require(count + size.toULong() <= value.receipt.encodedBytes)
                            provider.write(destination.fileDescriptor, buffer, size)
                            count += size.toULong()
                        }
                        require(count == value.receipt.encodedBytes)
                    }
                }
            }
            currentCoroutineContext().ensureActive()
            published = true
            value.receipt.fileReceipt()
            }
        } finally {
            if (!published) discard(uri)
        }
    }

    /** The ordinary Android share lease lasts 24 h. Only a checked PNG <=128 MiB
     * is copied here; larger files remain saveable through the document picker.
     * This is a per-file bound, not a claim about all other app share leases. */
    suspend fun share(binding: WorkflowBinding, options: ExportOptions,
        stage: (String) -> Unit): Pair<MediaHandoff.Lease, FileExportResult> = gate.withLock {
        val value = checked(binding, options)
        admitGiantShare(value.receipt.encodedBytes)
        settleShare()
        val owned = checkNotNull(work)
        try {
            stage("Copying the checked PNG into a read-only share lease…")
            withContext(Dispatchers.IO) {
                verifyFile(owned, value.receipt, value.privateSha256)
                require(availableBytes(owned.root) >= value.receipt.encodedBytes.toLong() + 1024L * 1024L)
                val handoff = handoffFactory(context)
                shareOwner = handoff
                pendingShare = handoff.create(camera = false)
                FileOutputStream(checkNotNull(pendingShare).file).use { output ->
                    owned.file("output.png").inputStream().use { input ->
                        val buffer = ByteArray(64 * 1024); var count = 0uL
                        while (true) {
                            ensureActive()
                            val size = input.read(buffer)
                            if (size < 0) break
                            if (size == 0) continue
                            require(count + size.toULong() <= value.receipt.encodedBytes)
                            output.write(buffer, 0, size); count += size.toULong()
                        }
                        require(count == value.receipt.encodedBytes)
                    }
                    output.fd.sync()
                }
            }
            currentCoroutineContext().ensureActive()
            val result = checkNotNull(pendingShare) to value.receipt.fileReceipt()
            pendingShare = null; shareOwner = null // Transfer without another suspension.
            result
        } finally { withContext(NonCancellable) { settleShare() } }
    }

    private fun checked(binding: WorkflowBinding, options: ExportOptions): Completed {
        val value = checkNotNull(completed) { "Check the giant PNG export before saving or sharing." }
        require(value.binding == binding && value.options == options) {
            "The project or export settings changed. Check the giant PNG again."
        }
        verifyGiantPngReceipt(binding, options, value.receipt)
        return value
    }

    /** Retained cleanup state is cleared only after successful deletion. Every
     * closer joins the serial export/publication work, even when cancelled. */
    suspend fun clear() = withContext(NonCancellable) { gate.withLock { clearLocked() } }
    private suspend fun clearLocked() {
        completed = null
        settleShare()
        val owned = work ?: return
        withContext(NonCancellable + Dispatchers.IO) { if (owned.isOpen) owned.close() }
        work = null
    }
    private suspend fun settleShare() {
        val lease = pendingShare ?: return
        val owner = checkNotNull(shareOwner)
        withContext(NonCancellable + Dispatchers.IO) { owner.remove(lease, camera = false) }
        pendingShare = null; shareOwner = null
    }
    private suspend fun discard(uri: Uri) = withContext(NonCancellable) {
        val removed = runCatching {
            withTimeout(2_000) { provider.perform {
                if (DocumentsContract.isDocumentUri(context, uri)) DocumentsContract.deleteDocument(context.contentResolver, uri)
                else context.contentResolver.delete(uri, null, null) > 0
            } }
        }.getOrDefault(false)
        if (!removed) throw RasterTransferFailure("The giant PNG was not saved completely. A partial file may remain at the selected destination; the private checked PNG and original are preserved.")
    }
    private suspend fun verifyFile(owned: RasterTransfers.Workspace, receipt: GiantPngReceipt, expected: ByteArray? = null): ByteArray {
        val output = owned.file("output.png")
        require(output.isFile && output.length().toULong() == receipt.encodedBytes)
        val digest = MessageDigest.getInstance("SHA-256")
        var total = 0uL
        output.inputStream().use { input ->
            val buffer = ByteArray(64 * 1024)
            while (true) {
                currentCoroutineContext().ensureActive()
                val size = input.read(buffer)
                if (size < 0) break
                if (size == 0) continue
                require(total + size.toULong() <= receipt.encodedBytes)
                digest.update(buffer, 0, size); total += size.toULong()
            }
        }
        require(total == receipt.encodedBytes)
        val actual = digest.digest()
        require(expected == null || MessageDigest.isEqual(expected, actual)) { "The private completed PNG changed. Check the export again." }
        return actual
    }
}

internal fun admitGiantShare(bytes: ULong) {
    require(bytes in 1uL..GIANT_PNG_SHARE_LIMIT.toULong()) {
        "This PNG exceeds the 128 MiB in-app share limit. Save it with the document picker, then share from Files."
    }
}

internal fun giantPngDimensions(binding: WorkflowBinding, width: UInt, height: UInt,
    options: ExportOptions): Pair<UInt, UInt> {
    require(options.documentId == binding.documentId && width > 0u && height > 0u)
    require(options.format == ImageFormat.Png8 && options.matteRgb == null && !options.convertToSrgb &&
        !options.allowDepthReduction && options.assumeUntaggedSrgb) {
        "Giant export currently preserves opaque untagged 8-bit pixels as PNG8. Confirm the sRGB assumption; depth, profile and matte conversions are not silently applied."
    }
    require(options.memoryBudgetBytes in 1uL..256uL * 1024uL * 1024uL)
    val region = options.region ?: return width to height
    require(listOf(region.x, region.y, region.width, region.height).all { it.isFinite() && it >= 0.0 && floor(it) == it })
    require(region.width > 0.0 && region.height > 0.0 && region.x + region.width <= width.toDouble() && region.y + region.height <= height.toDouble())
    return region.width.toUInt() to region.height.toUInt()
}

internal fun verifyGiantPngReceipt(binding: WorkflowBinding, options: ExportOptions, receipt: GiantPngReceipt) {
    val revision = receipt.revision
    require(options.documentId == binding.documentId && revision.projectId == binding.projectId &&
        revision.hostSeq == binding.hostSeq && revision.stateHash == binding.stateHash && binding.documentId in revision.documentIds) {
        "The native PNG receipt does not match the displayed project revision."
    }
    require(receipt.encodedBytes in 1uL..GIANT_PNG_FILE_LIMIT.toULong() && receipt.estimatedPeakBytes <= options.memoryBudgetBytes)
    require(receipt.blake3.length == 64 && receipt.blake3.all { it in '0'..'9' || it in 'a'..'f' } && receipt.metadataJson.length <= 64 * 1024)
}
private fun GiantPngReceipt.fileReceipt() = FileExportResult(encodedBytes, blake3, metadataJson, revision, estimatedPeakBytes)
