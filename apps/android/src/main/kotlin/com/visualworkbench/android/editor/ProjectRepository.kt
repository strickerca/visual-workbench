package com.visualworkbench.android.editor

import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import com.visualworkbench.shared.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.io.File
import java.text.DateFormat
import java.util.Date
import java.util.concurrent.atomic.AtomicLong
import kotlin.coroutines.coroutineContext

internal class ImageTooLarge : Exception("Images over 50 megapixels aren't supported yet")
internal data class LocalProject(val path: String, val info: ProjectInfo?, val modifiedMs: Long)

/** App-private project paths and install-random device identity, never hardware IDs. */
internal class ProjectRepository(context: Context, val core: WorkbenchCore) {
    private val context = context.applicationContext
    private val root = File(this.context.filesDir, "projects")
    private val rasterTransfers = RasterTransfers(this.context)
    private val next = AtomicLong(0)
    private var reservedUntil = 0L
    lateinit var deviceId: String
        private set

    suspend fun initialize() = withContext(Dispatchers.IO) {
        check(root.isDirectory || root.mkdirs())
        val preferences = context.getSharedPreferences("local-identity", Context.MODE_PRIVATE)
        deviceId = preferences.getString("device", null) ?: core.newDeviceId()
        val previous = preferences.getLong("lamport-reserved", 1)
        require(previous > 0 && previous <= Long.MAX_VALUE - 1_000_000) { "Device operation counter exhausted" }
        reservedUntil = previous + 1_000_000
        check(preferences.edit().putString("device", deviceId).putLong("lamport-reserved", reservedUntil).commit())
        next.set(previous)
    }

    // Reserve before any contact. Skipping unused values after restart is safe.
    // No storage write is on the pen input path.
    fun lamport(count: Int = 1): ULong {
        require(count in 1..512)
        val first = next.getAndAdd(count.toLong())
        check(first > 0 && first <= reservedUntil - count) { "Reopen the app to reserve another operation counter range" }
        return first.toULong()
    }

    suspend fun list(): List<LocalProject> = withContext(Dispatchers.IO) {
        root.listFiles()?.filter { it.isDirectory && it.name.matches(Regex("[0-9a-fA-F-]{36}")) && File(it, "project.sqlite").isFile }
            ?.sortedByDescending { it.lastModified() }?.take(1024)?.mapNotNull { folder ->
                coroutineContext.ensureActive()
                var project: WorkbenchProject? = null
                try {
                    project = core.open(checkedPath(folder.name))
                    LocalProject(folder.name, project.info(), File(folder, "project.sqlite").lastModified())
                } catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel }
                catch (_: CoreFailure) { LocalProject(folder.name, null, File(folder, "project.sqlite").lastModified()) }
                finally { withContext(NonCancellable) { project?.close() } }
            } ?: emptyList()
    }

    fun checkedPath(id: String): String {
        require(id.matches(Regex("[0-9a-fA-F-]{36}"))) { "Invalid local project ID" }
        val base = root.canonicalFile
        val target = File(base, id).canonicalFile
        require(target.parentFile == base) { "Project path escaped its root" }
        return target.absolutePath
    }

    suspend fun open(id: String): WorkbenchProject = core.open(checkedPath(id))

    /** The authenticated receiver publishes with no-clobber semantics. Folder
     * identity is local storage bookkeeping, independent of the remote ProjectId. */
    fun receivePath(): String = checkedPath(core.newId(System.currentTimeMillis().toULong()))

    suspend fun createBlank(): WorkbenchProject {
        val source = withContext(Dispatchers.IO) {
            val bitmap = Bitmap.createBitmap(1600, 1200, Bitmap.Config.ARGB_8888)
            try {
                bitmap.eraseColor(android.graphics.Color.WHITE)
                ByteArrayOutputStream().also { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }.toByteArray()
            } finally { bitmap.recycle() }
        }
        // Only managed data crosses the cancellable dispatcher boundary. The
        // facade's native ownership handoff must deliver directly to this caller.
        return create(source, "Untitled canvas")
    }

    suspend fun importImage(uri: Uri, stage: (String) -> Unit = {}): WorkbenchProject {
        val streaming = core as? WorkbenchStreamingCore ?: throw CoreFailure(CoreFailureKind.Unsupported)
        return rasterTransfers.importImage(uri, streaming, { source, work, title ->
            val now = System.currentTimeMillis()
            val project = core.newId(now.toULong())
            CreateFileProject(checkedPath(project), project, core.newId(now.toULong()), core.newId(now.toULong()), deviceId, title, source, work, now)
        }, stage)
    }

    private suspend fun create(bytes: ByteArray, title: String): WorkbenchProject {
        val now = System.currentTimeMillis()
        val project = core.newId(now.toULong())
        return core.create(CreateProject(checkedPath(project), project, core.newId(now.toULong()), core.newId(now.toULong()), deviceId, title, bytes, now))
    }

    suspend fun exportImage(uri: Uri, project: WorkbenchProject, options: ExportOptions, stage: (String) -> Unit): FileExportResult =
        rasterTransfers.export(uri, project as? WorkbenchStreamingProject ?: throw CoreFailure(CoreFailureKind.Unsupported), options, stage)

    suspend fun sharePng(project: WorkbenchProject, options: ExportOptions, stage: (String) -> Unit): Pair<MediaHandoff.Lease, FileExportResult> =
        rasterTransfers.sharePng(project as? WorkbenchStreamingProject ?: throw CoreFailure(CoreFailureKind.Unsupported), options, stage)

    suspend fun preflight(project: WorkbenchProject, options: ExportOptions): FilePreflight =
        rasterTransfers.preflight(project as? WorkbenchFileTransfers ?: throw CoreFailure(CoreFailureKind.Unsupported), options)

    suspend fun discardDestination(uri: Uri) { rasterTransfers.discardDestination(uri) }
    suspend fun saveCamera(source: File, uri: Uri, stage: (String) -> Unit) { rasterTransfers.saveOriginal(source, uri, stage) }

    companion object {
        fun modified(time: Long): String = "Saved ${DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(time))}"
    }
}
