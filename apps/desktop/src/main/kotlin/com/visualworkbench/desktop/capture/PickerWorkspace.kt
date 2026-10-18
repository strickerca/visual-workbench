package com.visualworkbench.desktop.capture

import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardOpenOption.*
import java.nio.file.attribute.BasicFileAttributes
import java.util.UUID

/** Exact-child ownership; no recursive deletion or adoption of crash leftovers.
 * Each mutation checks the token and the complete bounded child inventory. */
internal class PickerWorkspace private constructor(
    val path: Path, private val token: String, private val directoryKey: Any?,
    private val deleteEmptyDirectory: (Path) -> Unit,
) {
    private val marker: Path get() = path.resolve("owner-v1")
    private var removed = false
    private var markerRemoved = false
    fun release() {
        if (removed) return
        val retryWithoutMarker = markerRemoved
        if (!markerRemoved) {
            val entries = ownedEntries()
            entries.filter { it != marker }.forEach(Files::delete)
            Files.delete(marker)
            markerRemoved = true
        }
        // A failed final directory deletion must remain retryable without
        // adopting an unmarked replacement directory or any new child.
        if (Files.exists(path, NOFOLLOW_LINKS)) {
            // Some Windows JDK providers expose no fileKey. Normal token-owned
            // deletion still works; an unmarked retry must refuse without an
            // exact directory identity rather than disabling all capture.
            if (retryWithoutMarker) checkNotNull(directoryKey)
            requireSameDirectory()
            require(Files.list(path).use { it.findAny().isEmpty })
            deleteEmptyDirectory(path)
        }
        require(Files.notExists(path, NOFOLLOW_LINKS))
        removed = true
    }
    fun retain() {
        if (removed) return
        if (markerRemoved) { release(); return }
        val entries = ownedEntries()
        if (entries.none { it.fileName.toString() == "capture.png" }) { release(); return }
        val note = path.resolve("recovery.txt")
        if (entries.none { it == note }) write(note,
            "Unsaved Visual Workbench capture. Original: capture.png. No project was overwritten. Import this exact PNG to recover it.\n")
    }
    private fun ownedEntries(): List<Path> {
        requireSameDirectory()
        require(Files.isRegularFile(marker, NOFOLLOW_LINKS) && Files.size(marker) == token.length.toLong() &&
            Files.readString(marker) == token)
        return inventory(path)
    }
    private fun requireSameDirectory() {
        require(Files.isDirectory(path, NOFOLLOW_LINKS) && path.toRealPath() == path &&
            Files.readAttributes(path, BasicFileAttributes::class.java, NOFOLLOW_LINKS).fileKey() == directoryKey)
    }
    companion object {
        private val names = setOf("owner-v1", "capture.png", "recovery.txt")
        private const val MAX_FILE_BYTES = 64L * 1024 * 1024
        @Synchronized fun create(projects: Path, deleteEmptyDirectory: (Path) -> Unit = { Files.delete(it) }): PickerWorkspace {
            require(projects.isAbsolute && projects == projects.normalize() && projects.toRealPath() == projects)
            val root = projects.resolve(".picker-inbox-v1")
            if (!Files.exists(root, NOFOLLOW_LINKS)) Files.createDirectory(root)
            require(Files.isDirectory(root, NOFOLLOW_LINKS) && root.toRealPath() == root)
            val entries = Files.list(root).use { it.limit(9).toList() }
            require(entries.size < 8) { "Recover or discard old picker captures before starting another" }
            var bytes = 0L
            for (entry in entries) {
                require(Files.isDirectory(entry, NOFOLLOW_LINKS) && entry.toRealPath() == entry)
                // Inventory only; existing captures are never adopted or removed.
                for (file in inventory(entry)) bytes = Math.addExact(bytes, Files.size(file))
            }
            require(bytes <= 448L * 1024 * 1024)
            val path = Files.createDirectory(root.resolve(UUID.randomUUID().toString()))
            val token = UUID.randomUUID().toString()
            try {
                val key = Files.readAttributes(path, BasicFileAttributes::class.java, NOFOLLOW_LINKS).fileKey()
                write(path.resolve("owner-v1"), token)
                return PickerWorkspace(path, token, key, deleteEmptyDirectory)
            }
            catch (error: Throwable) {
                try { Files.deleteIfExists(path.resolve("owner-v1")); Files.deleteIfExists(path) }
                catch (cleanup: Throwable) { error.addSuppressed(cleanup) }
                throw error
            }
        }
        private fun inventory(path: Path): List<Path> {
            val entries = Files.list(path).use { it.limit(4).toList() }
            require(entries.size <= 3 && entries.all {
                it.fileName.toString() in names && Files.isRegularFile(it, NOFOLLOW_LINKS) &&
                    !Files.isSymbolicLink(it) && Files.size(it) in 0..MAX_FILE_BYTES
            })
            return entries
        }
        private fun write(path: Path, value: String) {
            val bytes = value.toByteArray(Charsets.UTF_8); require(bytes.size <= 2048)
            FileChannel.open(path, CREATE_NEW, WRITE).use { channel ->
                val buffer = ByteBuffer.wrap(bytes)
                while (buffer.hasRemaining()) channel.write(buffer)
                channel.force(true)
            }
        }
    }
}
