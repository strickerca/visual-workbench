package com.visualworkbench.desktop.capture

import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardOpenOption.*
import java.util.UUID

/** Exact-child ownership; no recursive deletion or adoption of crash leftovers. */
internal class PickerWorkspace private constructor(val path: Path, private val token: String) {
    private val marker: Path get() = path.resolve("owner-v1")
    fun release() {
        require(path.toRealPath() == path && Files.isRegularFile(marker, NOFOLLOW_LINKS) &&
            Files.size(marker) == token.length.toLong() && Files.readString(marker) == token)
        val entries = Files.list(path).use { it.limit(4).toList() }
        require(entries.size <= 3 && entries.all { it.fileName.toString() in setOf("owner-v1", "capture.png", "recovery.txt") &&
            Files.isRegularFile(it, NOFOLLOW_LINKS) && !Files.isSymbolicLink(it) })
        entries.filter { it != marker }.forEach(Files::delete)
        Files.delete(marker); Files.delete(path)
    }
    fun retain() {
        if (!Files.isRegularFile(path.resolve("capture.png"), NOFOLLOW_LINKS)) { release(); return }
        val note = path.resolve("recovery.txt")
        if (!Files.exists(note, NOFOLLOW_LINKS)) write(note, "Unsaved Visual Workbench capture. Original: capture.png. No project was overwritten. Import this exact PNG to recover it.\n")
    }
    companion object {
        fun create(projects: Path): PickerWorkspace {
            require(projects.isAbsolute && projects == projects.normalize() && projects.toRealPath() == projects)
            val root = projects.resolve(".picker-inbox-v1")
            if (!Files.exists(root, NOFOLLOW_LINKS)) Files.createDirectory(root)
            require(Files.isDirectory(root, NOFOLLOW_LINKS) && root.toRealPath() == root)
            val entries = Files.list(root).use { it.limit(9).toList() }
            require(entries.size < 8) { "Recover or discard old picker captures before starting another" }
            var bytes = 0L
            for (entry in entries) {
                require(Files.isDirectory(entry, NOFOLLOW_LINKS) && entry.toRealPath() == entry)
                val files = Files.list(entry).use { it.limit(4).toList() }
                require(files.size <= 3 && files.all { Files.isRegularFile(it, NOFOLLOW_LINKS) })
                for (file in files) { val size = Files.size(file); require(size in 0..64L*1024*1024); bytes = Math.addExact(bytes, size) }
            }
            require(bytes <= 448L * 1024 * 1024)
            val path = Files.createDirectory(root.resolve(UUID.randomUUID().toString())); val token = UUID.randomUUID().toString()
            try { write(path.resolve("owner-v1"), token); return PickerWorkspace(path, token) }
            catch (error: Throwable) { Files.deleteIfExists(path.resolve("owner-v1")); Files.deleteIfExists(path); throw error }
        }
        private fun write(path: Path, value: String) {
            val bytes = value.toByteArray(Charsets.UTF_8); require(bytes.size <= 2048)
            FileChannel.open(path, CREATE_NEW, WRITE).use { channel ->
                val buffer = ByteBuffer.wrap(bytes); while (buffer.hasRemaining()) channel.write(buffer); channel.force(true)
            }
        }
    }
}
