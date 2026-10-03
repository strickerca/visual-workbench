package com.visualworkbench.android.editor

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.MediaStore
import androidx.core.content.FileProvider
import java.io.File
import java.util.UUID
import kotlinx.coroutines.*

internal suspend fun cleanupHandoff(action: suspend () -> Unit, report: () -> Unit): Boolean = withContext(NonCancellable) {
    try { withContext(Dispatchers.IO) { action() }; true }
    catch (_: Exception) { withContext(Dispatchers.Main.immediate) { report() }; false }
}

/** Only these disposable handoff files are exposed, never project or native scratch roots. */
internal class MediaHandoff(context: Context, private val uriForFile: (Context, File) -> Uri = { owner, file ->
    FileProvider.getUriForFile(owner, "${owner.packageName}.images", file)
}) {
    private val context = context.applicationContext
    private val root = File(this.context.cacheDir.canonicalFile, "media-handoff")

    data class Lease(val token: String, val file: File, val uri: Uri)

    fun create(camera: Boolean): Lease {
        val token = UUID.randomUUID().toString()
        val parent = parent(camera)
        if (camera) check(parent.listFiles().orEmpty().size < 64) { "Save or discard retained camera captures before taking another photo." }
        val directory = File(parent, token)
        check(directory.mkdir()) { "Temporary image storage is unavailable" }
        try {
            val lease = resolve(token, camera)
            check(lease.file.createNewFile())
            return lease
        } catch (error: Exception) {
            // Roll back only the exact empty directory created above. Never
            // recurse or remove an unexpected file after a failed creation.
            runCatching {
                if (directory.canonicalFile == directory.absoluteFile && directory.parentFile == parent && directory.listFiles()?.isEmpty() == true)
                    check(directory.delete())
            }
            throw error
        }
    }

    fun resolve(token: String, camera: Boolean): Lease {
        require(token.matches(Regex("[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}")))
        val parent = parent(camera)
        val directory = File(parent, token)
        require(directory.canonicalFile == directory.absoluteFile && directory.parentFile == parent && directory.isDirectory)
        val file = File(directory, if (camera) "capture.jpg" else "image.png")
        require(file.canonicalFile == file.absoluteFile)
        return Lease(token, file, uriForFile(context, file))
    }

    fun remove(lease: Lease, camera: Boolean) {
        val checked = resolve(lease.token, camera)
        require(checked.file == lease.file && checked.uri == lease.uri)
        require(checked.file.parentFile?.listFiles()?.all { it == checked.file } == true)
        revoke(checked)
        check(!checked.file.exists() || checked.file.delete())
        check(checkNotNull(checked.file.parentFile).delete())
    }

    fun revoke(lease: Lease) { context.revokeUriPermission(lease.uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION) }

    /** Shares remain readable for a day. Cleanup never walks arbitrary subtrees or active captures. */
    fun prune(nowMs: Long, activeCamera: String? = null) {
        for (camera in listOf(false, true)) {
            for (directory in parent(camera).listFiles().orEmpty().take(1024)) {
                if (camera && directory.name == activeCamera) continue
                val lease = runCatching { resolve(directory.name, camera) }.getOrNull() ?: continue
                // A completed camera capture may be the only original. It is
                // durable until imported or explicitly saved/discarded.
                if (camera && lease.file.length() > 0) continue
                if (lease.file.isFile && nowMs - lease.file.lastModified() > RETENTION_MS) remove(lease, camera)
            }
        }
    }

    fun captures(): List<Lease> = parent(true).listFiles().orEmpty().take(64).mapNotNull { directory ->
        runCatching { resolve(directory.name, camera = true) }.getOrNull()?.takeIf { it.file.isFile && it.file.length() > 0 }
    }

    private fun parent(camera: Boolean): File {
        if (camera) {
            val files = context.filesDir.canonicalFile
            val inbox = File(files, "camera-inbox")
            check(inbox.isDirectory || inbox.mkdir())
            require(inbox.canonicalFile == inbox.absoluteFile && inbox.parentFile == files)
            return inbox
        }
        val cache = context.cacheDir.canonicalFile
        check(root.isDirectory || root.mkdir())
        require(root.canonicalFile == root.absoluteFile && root.parentFile?.canonicalFile == cache)
        val directory = File(root, "share")
        check(directory.isDirectory || directory.mkdir())
        require(directory.canonicalFile == directory.absoluteFile)
        return directory
    }

    companion object { const val RETENTION_MS = 24L * 60 * 60 * 1000 }
}

internal object MediaIntents {
    @Suppress("DEPRECATION")
    fun incoming(intent: Intent): Uri? {
        if (intent.action != Intent.ACTION_SEND || intent.type?.startsWith("image/") != true) return null
        val uri = intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM) ?: return null
        require(uri.scheme == "content") { "Choose an image from a content provider." }
        return uri
    }

    fun camera(uri: Uri): Intent = Intent(MediaStore.ACTION_IMAGE_CAPTURE)
        .putExtra(MediaStore.EXTRA_OUTPUT, uri)
        .apply { clipData = ClipData.newRawUri("Camera output", uri) }
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)

    fun sharePng(uri: Uri): Intent = Intent(Intent.ACTION_SEND)
        .setType("image/png")
        .putExtra(Intent.EXTRA_STREAM, uri)
        .apply { clipData = ClipData.newRawUri("Exported PNG", uri) }
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
}
