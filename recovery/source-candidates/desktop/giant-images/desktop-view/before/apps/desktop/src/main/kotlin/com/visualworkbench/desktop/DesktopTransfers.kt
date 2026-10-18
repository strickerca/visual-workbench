package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.*
import java.nio.file.attribute.BasicFileAttributes
import java.util.UUID
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.ceil
import kotlin.math.floor

enum class ExportArea(val label: String) { Full("Full image"), Selection("Selection bounds"), View("Visible view bounds") }
enum class ExportEncoding(val label: String, val extension: String) {
    Png8("PNG · 8 bit", "png"), Png16("PNG · 16 bit", "png"),
    Jpeg("JPEG", "jpg"), WebpLossless("WebP · lossless", "webp"), WebpLossy("WebP · lossy", "webp");
    val png: Boolean get() = this == Png8 || this == Png16
}
enum class ExportMatte(val label: String, val rgb: UInt?) { Preserve("Preserve alpha", null), White("White matte", 0xffffffu), Black("Black matte", 0u) }
data class ExportSettings(
    val marked: Boolean = true, val area: ExportArea = ExportArea.Full,
    val encoding: ExportEncoding = ExportEncoding.Png8, val quality: Int = 90,
    val matte: ExportMatte = ExportMatte.Preserve, val convertToSrgb: Boolean = false,
    val assumeUntaggedSrgb: Boolean = false, val allowDepthReduction: Boolean = false,
    val allowDibDepthReduction: Boolean = false, val assumeDibSrgb: Boolean = false,
) {
    fun format(): ImageFormat = when (encoding) {
        ExportEncoding.Png8 -> ImageFormat.Png8
        ExportEncoding.Png16 -> ImageFormat.Png16
        ExportEncoding.Jpeg -> ImageFormat.Jpeg(quality.coerceIn(1, 100).toUByte())
        ExportEncoding.WebpLossless -> ImageFormat.WebpLossless
        ExportEncoding.WebpLossy -> ImageFormat.WebpLossy(quality.coerceIn(1, 100).toUByte())
    }
}

/** Immutable before opening a save picker or queueing a native operation. */
data class ExportRequest(
    val epoch: Long, val projectId: String, val documentId: String,
    val hostSeq: ULong, val stateHash: String, val settings: ExportSettings,
    val region: Rect?, val width: UInt, val height: UInt,
    val selection: Set<String>, val camera: Camera?,
) {
    val revision: String get() = "r$hostSeq-${stateHash.take(8)}"
    fun options(): ExportOptions = ExportOptions(documentId, settings.format(), settings.marked, region,
        settings.matte.rgb, settings.convertToSrgb, settings.assumeUntaggedSrgb, settings.allowDepthReduction)
    fun matches(state: EditorState): Boolean {
        val document = state.document ?: return false
        val info = document.render.revision
        return epoch == state.projectEpoch && projectId == info.projectId && documentId == document.documentId &&
            hostSeq == info.hostSeq && stateHash == info.stateHash && settings == state.exportSettings &&
            (settings.area != ExportArea.Selection || selection == state.selected) &&
            (settings.area != ExportArea.View || camera == state.view.camera)
    }
    fun matches(info: ProjectInfo): Boolean = projectId == info.projectId && hostSeq == info.hostSeq && stateHash == info.stateHash
}

data class CompletedExport(val request: ExportRequest, val bytes: ULong, val blake3: String,
    val sourceAsset: String, val revision: String, val width: UInt, val height: UInt)
data class ExportCheck(val request: ExportRequest, val result: FilePreflight)
data class ImportRequest(val epoch: Long, val path: Path)

internal fun captureExport(state: EditorState, core: WorkbenchCore): ExportRequest {
    val document = state.document ?: throw HandoffFailure("Open a document before exporting.")
    val settings = state.exportSettings
    val candidate = when (settings.area) {
        ExportArea.Full -> null
        ExportArea.Selection -> union(document.render.items.filter { it.objectId in state.selected }.map { it.bounds })
            ?: throw HandoffFailure("Select at least one object for its rectangular bounds, or choose the full image or view.")
        ExportArea.View -> {
            val c = state.view.camera
            val points = core.mapPoints(c, true, listOf(Point(0.0, 0.0), Point(c.viewportWidth, 0.0),
                Point(0.0, c.viewportHeight), Point(c.viewportWidth, c.viewportHeight)))
            if (points.size != 4) throw HandoffFailure("The view rectangle is unavailable.")
            val x = points.minOf { it.x }; val y = points.minOf { it.y }
            Rect(x, y, points.maxOf { it.x } - x, points.maxOf { it.y } - y)
        }
    }
    val region = candidate?.let { clippedExportRect(it, document.width, document.height) }
    val info = document.render.revision
    return ExportRequest(state.projectEpoch, info.projectId, document.documentId, info.hostSeq, info.stateHash,
        settings, region, region?.width?.toUInt() ?: document.width, region?.height?.toUInt() ?: document.height,
        if (settings.area == ExportArea.Selection) state.selected.toSet() else emptySet(),
        state.view.camera.takeIf { settings.area == ExportArea.View })
}
internal fun clippedExportRect(rect: Rect, width: UInt, height: UInt): Rect {
    if (listOf(rect.x, rect.y, rect.width, rect.height, rect.x + rect.width, rect.y + rect.height).any { !it.isFinite() } || rect.width <= 0 || rect.height <= 0)
        throw HandoffFailure("The export region is empty or invalid.")
    val left = floor(rect.x).coerceIn(0.0, width.toDouble())
    val top = floor(rect.y).coerceIn(0.0, height.toDouble())
    val right = ceil(rect.x + rect.width).coerceIn(0.0, width.toDouble())
    val bottom = ceil(rect.y + rect.height).coerceIn(0.0, height.toDouble())
    if (right <= left || bottom <= top) throw HandoffFailure("The selected region does not intersect the image.")
    return Rect(left, top, right - left, bottom - top)
}

internal class HandoffFailure(message: String) : IOException(message)
internal class RetainedDropCancellation(val input: Path) : CancellationException("Accepted drop retained for recovery")
internal fun transferMessage(error: Exception): String = when (error) {
    is HandoffFailure -> error.message ?: "The handoff was refused."
    is FileAlreadyExistsException -> "That file already exists. Choose a new name; the existing file was preserved."
    is TransferFailure -> when (error.kind) {
        TransferFailureKind.Dimensions -> "${error.width} × ${error.height} exceeds this format's ${error.dimensionLimit}-pixel side limit. Choose PNG or a smaller region. Nothing was resized."
        TransferFailureKind.PixelLimit -> "Images over 50 MP are not supported yet. The original was preserved and was not downsampled."
        TransferFailureKind.Memory -> "This export needs ${error.estimatedBytes?.div(1024uL * 1024uL)} MiB; its budget is ${error.budgetBytes?.div(1024uL * 1024uL)} MiB. Choose PNG or a smaller region."
        TransferFailureKind.Metadata -> "The color profile cannot be preserved in this export. Choose explicit sRGB conversion; untagged input also needs the sRGB assumption."
        TransferFailureKind.Alpha -> "This format cannot retain transparency. Choose a white or black matte, or PNG/WebP."
        TransferFailureKind.Depth -> "This format would reduce sample depth. Choose PNG 16 bit or explicitly allow depth reduction."
        TransferFailureKind.EncodedLimit -> "The encoded file exceeds the transfer byte limit. Choose a different format or region."
        TransferFailureKind.ScratchLimit -> "The export exceeds its 400 MB scratch budget. Choose a smaller region."
        TransferFailureKind.Backpressure -> "The native transfer queue is busy. Retry after current work finishes."
        TransferFailureKind.Unsupported -> "This source or format is not supported by the current transfer pipeline."
        TransferFailureKind.Invalid -> "The transfer request is invalid or its source changed. Prepare it again."
        TransferFailureKind.Storage -> "The transfer could not safely read or publish its file. The existing destination was preserved."
        TransferFailureKind.Cancelled -> "Transfer cancelled."
        TransferFailureKind.Closed -> "The project closed before this transfer finished."
    }
    else -> "The handoff could not finish. The original and existing destination were preserved."
}

/** These fields come from bounded canonical core JSON, not arbitrary app text.
 * No new JSON dependency or parsing of embedded PNG bytes is needed here; the
 * host independently verifies the complete PNG and its embedded receipt. */
internal fun completeExport(request: ExportRequest, result: FileExportResult): CompletedExport {
    if (!request.matches(result.revision) || result.metadataJson.length > 65536 ||
        !Regex("[0-9a-f]{64}").matches(result.blake3) || result.encodedBytes == 0uL)
        throw HandoffFailure("The export receipt does not match the captured revision.")
    fun string(key: String): String = Regex("\"$key\"\\s*:\\s*\"([^\"]*)\"").findAll(result.metadataJson)
        .toList().singleOrNull()?.groupValues?.get(1) ?: throw HandoffFailure("The export receipt is incomplete.")
    fun number(key: String): UInt = Regex("\"$key\"\\s*:\\s*([0-9]+)(?=\\s*[,}])").findAll(result.metadataJson)
        .toList().singleOrNull()?.groupValues?.get(1)?.toUIntOrNull() ?: throw HandoffFailure("The export dimensions are missing.")
    val revision = string("revision"); val source = string("source_asset")
    val width = number("output_width"); val height = number("output_height")
    if (revision != request.revision || !Regex("[0-9a-f]{64}").matches(source) || width != request.width || height != request.height)
        throw HandoffFailure("The completed export has different source/revision or dimensions. Prepare it again.")
    return CompletedExport(request, result.encodedBytes, result.blake3, source, revision, width, height)
}

internal const val IMPORT_BYTES: Long = 64L * 1024L * 1024L
internal const val EXPORT_BYTES: Long = 512L * 1024L * 1024L

/** All byte/file work runs in IO. Each workspace has a random owner marker;
 * cleanup verifies ownership and containment and never follows links. */
internal class TransferFiles(private val base: Path) {
    suspend fun create(): TransferWorkspace {
        currentCoroutineContext().ensureActive()
        val owned = AtomicReference<TransferWorkspace?>(null)
        try {
            withContext(NonCancellable + Dispatchers.IO) {
                val root = base.toAbsolutePath().normalize()
                Files.createDirectories(root); requirePlainDirectory(root)
                val directory = Files.createTempDirectory(root, "handoff-")
                val token = UUID.randomUUID().toString()
                val marker = directory.resolve("owner")
                var markerOwned = false
                try {
                    FileChannel.open(marker, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { channel ->
                        markerOwned = true
                        val bytes = ByteBuffer.wrap(token.toByteArray(Charsets.UTF_8))
                        while (bytes.hasRemaining()) channel.write(bytes)
                        channel.force(true)
                    }
                    TransferWorkspace(root, directory, token).also { owned.set(it) }
                } catch (error: Throwable) {
                    // Only our successful CREATE_NEW grants marker ownership.
                    // Unexpected entries leave a nonempty directory preserved.
                    try {
                        if (markerOwned) Files.deleteIfExists(marker)
                        Files.deleteIfExists(directory)
                    } catch (cleanup: Exception) { error.addSuppressed(cleanup) }
                    throw error
                }
            }
            currentCoroutineContext().ensureActive()
            return owned.getAndSet(null) ?: throw HandoffFailure("Private transfer staging was not created.")
        } catch (error: Throwable) { owned.getAndSet(null)?.close(); throw error }
    }
}
internal class TransferWorkspace(private val root: Path, val directory: Path, private val token: String) {
    val input: Path get() = directory.resolve("input")
    val output: Path get() = directory.resolve("output")
    suspend fun stage(source: Path, limit: Long = IMPORT_BYTES): Path = withContext(Dispatchers.IO) {
        if (!source.isAbsolute) throw HandoffFailure("Choose an absolute local file path.")
        val before = Files.readAttributes(source, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
        if (!before.isRegularFile || before.isSymbolicLink || before.size() !in 1..limit)
            throw HandoffFailure("Import requires a regular file of at most ${limit / 1024 / 1024} MiB. Images over 50 MP are refused.")
        requirePlainDirectory(source.parent)
        copyFile(source, input, limit)
        val after = Files.readAttributes(source, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
        if (before.fileKey() != after.fileKey() || before.size() != after.size() || before.lastModifiedTime() != after.lastModifiedTime())
            throw HandoffFailure("The source changed during import. Choose it again after it has finished saving.")
        input
    }
    suspend fun stage(bytes: ByteArray): Path = withContext(Dispatchers.IO) {
        if (bytes.isEmpty() || bytes.size.toLong() > IMPORT_BYTES) throw HandoffFailure("Clipboard image exceeds its import byte limit.")
        FileChannel.open(input, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { channel ->
            val buffer = ByteBuffer.wrap(bytes)
            while (buffer.hasRemaining()) { currentCoroutineContext().ensureActive(); channel.write(buffer) }
            channel.force(true)
        }
        input
    }
    /** Written and flushed before an accepted drop permits the source app to
     * remove its temporary original. On later failure the complete workspace
     * remains available for deliberate recovery; no automatic orphan deletion. */
    suspend fun markAcceptedDrop(sourceName: String): Unit = withContext(Dispatchers.IO) {
        requirePlainDirectory(root); requirePlainDirectory(directory)
        val info = Files.readAttributes(input, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
        if (!info.isRegularFile || info.isSymbolicLink || info.size() !in 1..IMPORT_BYTES)
            throw HandoffFailure("The staged drop could not be retained safely.")
        val name = sourceName.take(256).map { if (it.isISOControl()) '_' else it }.joinToString("")
        val note = "Visual Workbench drop recovery v1\nOriginal file name: $name\nOriginal bytes: ${info.size()}\n" +
            "The original encoded bytes are in input. Copy it using the original file name and extension, then import that copy to retry.\n" +
            "This directory is preserved after an accepted drop fails or is cancelled.\n"
        val bytes = note.toByteArray(Charsets.UTF_8)
        if (bytes.size > 2048) throw HandoffFailure("The drop recovery marker exceeded its bound.")
        FileChannel.open(directory.resolve("drop-recovery.txt"), StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { channel ->
            val buffer = ByteBuffer.wrap(bytes)
            while (buffer.hasRemaining()) { currentCoroutineContext().ensureActive(); channel.write(buffer) }
            channel.force(true)
        }
    }
    suspend fun close(): Unit = withContext(NonCancellable + Dispatchers.IO) {
        if (!Files.exists(directory, LinkOption.NOFOLLOW_LINKS)) return@withContext
        requirePlainDirectory(root); requirePlainDirectory(directory)
        val marker = directory.resolve("owner")
        val markerInfo = Files.readAttributes(marker, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
        if (directory.parent != root || !directory.fileName.toString().startsWith("handoff-") ||
            !markerInfo.isRegularFile || markerInfo.isSymbolicLink || markerInfo.size() != token.length.toLong())
            throw HandoffFailure("Private transfer ownership changed; cleanup preserved the directory.")
        val markerBytes = ByteBuffer.allocate(token.length + 1)
        FileChannel.open(marker, StandardOpenOption.READ, LinkOption.NOFOLLOW_LINKS).use { channel ->
            while (markerBytes.hasRemaining() && channel.read(markerBytes) >= 0) { }
        }
        if (markerBytes.position() != token.length ||
            String(markerBytes.array(), 0, markerBytes.position(), Charsets.UTF_8) != token)
            throw HandoffFailure("Private transfer ownership changed; cleanup preserved the directory.")
        Files.walkFileTree(directory, object : SimpleFileVisitor<Path>() {
            override fun visitFile(file: Path, attributes: BasicFileAttributes): FileVisitResult {
                if (!file.normalize().startsWith(directory)) throw HandoffFailure("Cleanup containment failed.")
                Files.delete(file); return FileVisitResult.CONTINUE
            }
            override fun postVisitDirectory(dir: Path, error: IOException?): FileVisitResult {
                if (error != null) throw error
                if (!dir.normalize().startsWith(directory)) throw HandoffFailure("Cleanup containment failed.")
                Files.delete(dir); return FileVisitResult.CONTINUE
            }
        })
    }
}
internal fun requirePlainDirectory(path: Path) {
    val normalized = path.toAbsolutePath().normalize()
    if (normalized.toRealPath() != normalized || !Files.isDirectory(normalized, LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(normalized))
        throw HandoffFailure("Linked or redirected transfer directories are not supported. Choose a plain local directory.")
}
internal suspend fun copyFile(source: Path, destination: Path, limit: Long): Long {
    var count = 0L
    FileChannel.open(source, StandardOpenOption.READ, LinkOption.NOFOLLOW_LINKS).use { input ->
        FileChannel.open(destination, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { output ->
            val buffer = ByteBuffer.allocate(64 * 1024)
            while (true) {
                currentCoroutineContext().ensureActive(); buffer.clear()
                val n = input.read(buffer); if (n < 0) break
                count = Math.addExact(count, n.toLong())
                if (count > limit) throw HandoffFailure("The file exceeded the transfer byte limit.")
                buffer.flip(); while (buffer.hasRemaining()) output.write(buffer)
            }
            output.force(true)
        }
    }
    return count
}

/** A complete sibling file is atomically linked into the chosen name. Hard-link
 * creation is no-clobber; unsupported volumes fail without a partial destination.
 * ATOMIC_MOVE is deliberately avoided because its existing-target behavior is
 * platform-dependent. The user can always choose another local destination. */
internal suspend fun publishExport(source: Path, target: Path, expected: ULong, current: () -> Boolean): Unit = withContext(Dispatchers.IO) {
    if (!target.isAbsolute || expected == 0uL || expected > EXPORT_BYTES.toULong()) throw HandoffFailure("Invalid export destination or byte count.")
    val destination = target.normalize(); val parent = destination.parent ?: throw HandoffFailure("Choose a destination directory.")
    requirePlainDirectory(parent)
    if (Files.exists(destination, LinkOption.NOFOLLOW_LINKS)) throw FileAlreadyExistsException(destination.toString())
    val temporary = parent.resolve(".vw-export-${UUID.randomUUID()}.tmp")
    var owned = false; var published = false
    try {
        // CREATE_NEW owns this exact random path; never delete a preexisting entry.
        FileChannel.open(temporary, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { }
        owned = true
        FileChannel.open(source, StandardOpenOption.READ, LinkOption.NOFOLLOW_LINKS).use { input ->
            FileChannel.open(temporary, StandardOpenOption.WRITE).use { output ->
                val buffer = ByteBuffer.allocate(64 * 1024); var size = 0uL
                while (true) {
                    currentCoroutineContext().ensureActive(); buffer.clear()
                    val n = input.read(buffer); if (n < 0) break
                    size += n.toULong(); if (size > expected) throw HandoffFailure("The prepared export changed.")
                    buffer.flip(); while (buffer.hasRemaining()) output.write(buffer)
                }
                if (size != expected) throw HandoffFailure("The prepared export changed.")
                output.force(true)
            }
        }
        currentCoroutineContext().ensureActive()
        if (!current()) throw HandoffFailure("The document, region or export settings changed. Prepare the export again.")
        try { Files.createLink(destination, temporary); published = true }
        catch (error: FileAlreadyExistsException) { throw error }
        catch (error: UnsupportedOperationException) { throw HandoffFailure("This destination cannot publish a complete file atomically. Choose a local NTFS directory.") }
    } finally {
        if (owned) try { Files.deleteIfExists(temporary) }
        catch (_: IOException) {
            throw HandoffFailure(if (published) "The complete export was saved, but its owned temporary file could not be removed. The saved file remains intact."
                else "No destination was published, but the owned temporary file could not be removed.")
        }
    }
}
