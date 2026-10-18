package com.visualworkbench.android.editor

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import java.io.File
import java.io.FileOutputStream
import java.util.UUID
import kotlin.coroutines.coroutineContext

internal class RasterTransferFailure(message: String, cause: Throwable? = null) : Exception(message, cause)

/** Only small buffers cross Android's content provider boundary. Source pixels,
 * ICC/orientation handling and PNG encoding stay in the shared Rust core. */
internal class RasterTransfers(context: Context, private val handoffFactory: (Context) -> MediaHandoff = { MediaHandoff(it) }) {
    private val context = context.applicationContext
    private val provider = ProviderIo(this.context.contentResolver)

    internal class Workspace(private val cacheRoot: File) {
        private val name = "raster-${UUID.randomUUID()}"
        private var directory: File? = null
        val root: File get() = checkNotNull(directory) { "Raster workspace is closed" }
        val isOpen: Boolean get() = directory != null

        fun create() {
            check(directory == null)
            val parent = File(cacheRoot.canonicalFile, "raster-transfers")
            check(parent.isDirectory || parent.mkdirs()) { "Temporary storage is unavailable" }
            val canonical = parent.canonicalFile
            require(canonical.parentFile == cacheRoot.canonicalFile) { "Temporary storage path changed" }
            val candidate = File(canonical, name)
            check(candidate.mkdir()) { "Could not create a private raster workspace" }
            directory = candidate.canonicalFile
            require(root.parentFile == canonical)
        }

        fun file(name: String): File {
            require(name in setOf("original.bin", "output.png", "output.jpg", "output.webp"))
            val target = File(root, name)
            require(target.canonicalFile.parentFile == root)
            return target
        }

        fun close() {
            val owned = directory ?: return
            val parent = File(cacheRoot.canonicalFile, "raster-transfers").canonicalFile
            require(owned.canonicalFile == owned && owned.parentFile == parent && owned.name == name)
            for (child in owned.listFiles() ?: error("Could not inspect temporary raster files")) {
                // Native scratch/output files are direct children. Do not follow
                // unexpected links or recursively remove another directory.
                require(child.canonicalFile.parentFile == owned && child.isFile) { "Unexpected raster workspace entry" }
                check(child.delete()) { "Could not remove a temporary raster file" }
            }
            check(owned.delete()) { "Could not remove temporary raster workspace" }
            directory = null
        }
    }

    suspend fun importImage(
        uri: Uri, streaming: WorkbenchStreamingCore,
        options: (sourcePath: String, workDirectory: String, title: String) -> CreateFileProject,
        stage: (String) -> Unit,
    ): WorkbenchProject {
        val work = Workspace(context.cacheDir)
        var handle: WorkbenchProject? = null
        try {
            stage("Copying the original image…")
            val title = withContext(Dispatchers.IO) {
                work.create()
                val original = work.file("original.bin")
                copyOriginal(uri, original)
                provider.title(uri)
            }
            stage("Reading the full-resolution original…")
            handle = streaming.createFile(options(work.file("original.bin").absolutePath, work.root.absolutePath, title))
            // Cleanup may suspend. Keep ownership until it finishes and caller
            // cancellation is checked, then return without another suspension.
            withContext(NonCancellable + Dispatchers.IO) { work.close() }
            coroutineContext.ensureActive()
            val result = checkNotNull(handle)
            handle = null
            return result
        } finally {
            if (handle != null || work.isOpen) withContext(NonCancellable) {
                try { handle?.close() }
                finally { if (work.isOpen) withContext(Dispatchers.IO) { work.close() } }
            }
        }
    }

    suspend fun export(uri: Uri, project: WorkbenchStreamingProject, options: ExportOptions, stage: (String) -> Unit): FileExportResult {
        require(uri.scheme == "content") { "The export destination must be a document provider." }
        val work = Workspace(context.cacheDir)
        var published = false
        try {
            stage("Rendering the full-resolution image…")
            withContext(Dispatchers.IO) { work.create() }
            val output = work.file(outputName(options.format))
            val request = FileExportOptions(options, output.absolutePath, work.root.absolutePath)
            val result = if (project is WorkbenchFileTransfers) project.exportTransfer(request) else project.exportFile(request)
            stage("Saving the exported image…")
            withContext(Dispatchers.IO) {
                require(output.isFile && output.length().toULong() == result.encodedBytes) { "The completed export did not match its native receipt" }
                provider.open(uri, "wt").use { target ->
                    provider.nonBlocking(target)
                    output.inputStream().use { source ->
                        val buffer = ByteArray(64 * 1024)
                        var copied = 0uL
                        while (true) {
                            ensureActive()
                            val size = source.read(buffer)
                            if (size < 0) break
                            if (size == 0) continue
                            provider.write(target.fileDescriptor, buffer, size)
                            copied += size.toULong()
                        }
                        require(copied == result.encodedBytes) { "The completed export changed during saving" }
                    }
                }
            }
            coroutineContext.ensureActive()
            published = true
            return result
        } finally {
            withContext(NonCancellable) {
                try { if (work.isOpen) withContext(Dispatchers.IO) { work.close() } }
                finally {
                    // The URI comes from CreateDocument, so this operation owns
                    // that newly created destination. Remove incomplete output.
                    if (!published) withContext(Dispatchers.IO) {
                        val deleted = deleteProviderDestination(uri)
                        if (!deleted) throw RasterTransferFailure("The export did not finish. A partial file may remain at the selected location; your original and edits are intact.")
                    }
                }
            }
        }
    }

    suspend fun preflight(project: WorkbenchFileTransfers, options: ExportOptions): FilePreflight {
        val work = Workspace(context.cacheDir)
        try {
            withContext(Dispatchers.IO) { work.create() }
            return project.preflightFile(FileExportOptions(options, work.file(outputName(options.format)).absolutePath, work.root.absolutePath))
        } finally { withContext(NonCancellable + Dispatchers.IO) { if (work.isOpen) work.close() } }
    }

    private fun outputName(format: ImageFormat): String = when (format) {
        ImageFormat.Png8, ImageFormat.Png16 -> "output.png"
        is ImageFormat.Jpeg -> "output.jpg"
        ImageFormat.WebpLossless, is ImageFormat.WebpLossy -> "output.webp"
    }

    suspend fun discardDestination(uri: Uri) = withContext(NonCancellable + Dispatchers.IO) {
        require(uri.scheme == "content") { "The export destination must be a document provider." }
        val deleted = deleteProviderDestination(uri)
        if (!deleted) throw RasterTransferFailure("The unused export destination could not be removed. An empty file may remain; the original and edits are intact.")
    }

    private suspend fun deleteProviderDestination(uri: Uri): Boolean = runCatching {
        withTimeout(2_000) { provider.perform {
            if (DocumentsContract.isDocumentUri(context, uri)) DocumentsContract.deleteDocument(context.contentResolver, uri)
            else context.contentResolver.delete(uri, null, null) > 0
        } }
    }.getOrDefault(false)

    suspend fun saveOriginal(source: File, uri: Uri, stage: (String) -> Unit) {
        require(uri.scheme == "content")
        var complete = false
        try {
            stage("Saving the retained camera original…")
            withContext(Dispatchers.IO) {
                provider.open(uri, "wt").use { output ->
                    provider.nonBlocking(output)
                    val expected = source.length(); var total = 0L
                    source.inputStream().use { input ->
                        val buffer = ByteArray(64 * 1024)
                        while (true) {
                            ensureActive()
                            val size = input.read(buffer)
                            if (size < 0) break
                            if (size == 0) continue
                            provider.write(output.fileDescriptor, buffer, size); total += size
                        }
                    }
                    require(total == expected)
                }
            }
            coroutineContext.ensureActive(); complete = true
        } finally { if (!complete) discardDestination(uri) }
    }

    suspend fun sharePng(project: WorkbenchStreamingProject, options: ExportOptions, stage: (String) -> Unit): Pair<MediaHandoff.Lease, FileExportResult> {
        require(options.format == ImageFormat.Png8 || options.format == ImageFormat.Png16)
        val work = Workspace(context.cacheDir)
        val handoff = handoffFactory(context)
        var lease: MediaHandoff.Lease? = null
        var delivered = false
        try {
            stage("Preparing a full-resolution PNG to share…")
            withContext(Dispatchers.IO) { work.create() }
            val output = work.file("output.png")
            val request = FileExportOptions(options, output.absolutePath, work.root.absolutePath)
            val result = if (project is WorkbenchFileTransfers) project.exportTransfer(request) else project.exportFile(request)
            withContext(Dispatchers.IO) {
                require(output.isFile && output.length().toULong() == result.encodedBytes)
                lease = handoff.create(camera = false)
                FileOutputStream(checkNotNull(lease).file).use { target ->
                    output.inputStream().use { source ->
                        val buffer = ByteArray(64 * 1024)
                        var count = 0uL
                        while (true) {
                            ensureActive()
                            val read = source.read(buffer)
                            if (read < 0) break
                            if (read == 0) continue
                            target.write(buffer, 0, read); count += read.toULong()
                        }
                        require(count == result.encodedBytes)
                    }
                    target.fd.sync()
                }
            }
            withContext(NonCancellable + Dispatchers.IO) { work.close() }
            coroutineContext.ensureActive()
            val value = checkNotNull(lease) to result
            delivered = true
            return value
        } finally {
            withContext(NonCancellable + Dispatchers.IO) {
                try { if (work.isOpen) work.close() }
                finally { if (!delivered) lease?.let { handoff.remove(it, camera = false) } }
            }
        }
    }

    private suspend fun copyOriginal(uri: Uri, destination: File) {
        check(destination.createNewFile())
        provider.open(uri, "r").use { input ->
            provider.nonBlocking(input)
            FileOutputStream(destination).use { output ->
                val buffer = ByteArray(64 * 1024)
                var total = 0L
                while (true) {
                    coroutineContext.ensureActive()
                    val size = provider.read(input.fileDescriptor, buffer)
                    if (size == 0) break
                    total += size
                    if (total > 64L * 1024 * 1024) throw RasterTransferFailure("The encoded original exceeds the current 64 MiB input limit. Choose a smaller encoded file; the image was not resized.")
                    output.write(buffer, 0, size)
                }
                require(total > 0) { "The selected image is empty" }
                output.fd.sync()
            }
        }
    }
}
