package com.visualworkbench.desktop

import com.sun.jna.Native
import com.sun.jna.Memory
import com.sun.jna.Pointer
import com.sun.jna.WString
import com.sun.jna.win32.StdCallLibrary
import java.io.InputStream
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.*
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean

internal fun interface NativeResources { fun open(name: String): InputStream? }
internal interface NativeFileGuard {
    fun check(path: Path)
    /** Production pins deny writes AND deletion/renaming for the JVM lifetime. */
    fun pin(path: Path): AutoCloseable
    fun pinDirectory(path: Path): AutoCloseable
}
internal class NativeRuntimeFailure(message: String) : Exception(message)

/** Runs before the first generated binding, DPI call or AWT/Compose window.
 * The two DLLs and helper come from the same build's recorded resource hashes.
 * A cache is published with a final readiness marker, never by overwriting an
 * existing file. Windows DLLs may remain loaded after close: close releases
 * our read leases only and deliberately does not remove cache directories. */
internal class DesktopNativeRuntime private constructor(val directory: Path, private val leases: List<AutoCloseable>, private val packaged: Map<String,String>) : AutoCloseable {
    private val closed = AtomicBoolean(false)
    fun packagedHash(name: String): String = packaged[name] ?: throw NativeRuntimeFailure("Unknown packaged native helper")
    fun activate() {
        check(!closed.get())
        // This property belongs only to this JVM. Include the prior path after
        // the exact verified runtime directory, preserving unrelated libraries.
        val previous = System.getProperty("jna.library.path").orEmpty()
        System.setProperty("jna.library.path", directory.toString() + if (previous.isBlank()) "" else java.io.File.pathSeparator + previous)
        // UniFFI 0.32.2 NamespaceLibraryTemplate uses the actual Rust library
        // namespace for these properties. Absolute names avoid name-based DLL
        // or classpath fallback to a different build.
        System.setProperty("uniffi.component.vw_core.libraryOverride", directory.resolve("vw_core.dll").toString())
        System.setProperty("uniffi.component.vw_host.libraryOverride", directory.resolve("vw_host.dll").toString())
    }
    override fun close() {
        if (closed.compareAndSet(false, true)) leases.asReversed().forEach { runCatching { it.close() } }
    }
    companion object {
        private val names = setOf("vw_core.dll", "vw_host.dll", "vw-connection-helper.exe", "vw-capture-helper.exe")
        private const val MAX_FILE = 256L * 1024 * 1024
        private const val MAX_TOTAL = 512L * 1024 * 1024
        private const val MANIFEST = "vw-native-runtime.sha256"
        private const val OWNER = "VisualWorkbench native runtime v1\n"

        fun prepare(localAppData: Path): DesktopNativeRuntime = prepareUnderAppData(
            localAppData,
            NativeResources { DesktopNativeRuntime::class.java.classLoader.getResourceAsStream(it) }, WindowsNativeFileGuard())

        internal fun prepareUnderAppData(localAppData: Path, resources: NativeResources,
                                         guard: NativeAnchorGuard): DesktopNativeRuntime {
            val anchor = NativeCacheAnchor.prepare(localAppData, guard)
            try {
                // Extraction still checks and pins every physical ancestor.
                // The original namespace and anchor leases outlive that check.
                val runtime = prepareAt(anchor.directory.resolve("native-runtime"), resources, guard)
                try {
                    anchor.verify()
                    return DesktopNativeRuntime(runtime.directory, listOf(anchor, runtime), runtime.packaged)
                } catch (error: Exception) { runtime.close(); throw error }
            } catch (error: Exception) { anchor.close(); throw error }
        }

        internal fun prepareAt(base: Path, resources: NativeResources, guard: NativeFileGuard): DesktopNativeRuntime {
            val pinned = mutableListOf<AutoCloseable>()
            var created: Path? = null
            val written = mutableListOf<CreatedFile>()
            try {
                val manifest = resources.open(MANIFEST)?.use { it.readNBytes(4097) }
                    ?: throw NativeRuntimeFailure("The packaged native runtime manifest is missing.")
                val entries = parseManifest(manifest)
                val root = base.toAbsolutePath().normalize()
                ensureDirectories(root, guard, pinned)
                val key = digest(manifest)
                val directory = root.resolve(key)
                requireContained(root, directory)
                if (!Files.exists(directory, LinkOption.NOFOLLOW_LINKS)) {
                    Files.newDirectoryStream(root).use { entriesInRoot ->
                        val iterator = entriesInRoot.iterator(); var count = 0
                        while (iterator.hasNext()) { iterator.next(); count++; if (count >= 32) throw NativeRuntimeFailure("The owned native runtime cache is full. Close all app instances before reviewing old marked caches.") }
                    }
                    // CREATE_DIRECTORY is the ownership claim. A racing owner
                    // can win, but this attempt will never replace its files.
                    try { Files.createDirectory(directory); created = directory }
                    catch (_: FileAlreadyExistsException) { }
                    guard.check(directory)
                    if (created != null) {
                        pinned += guard.pinDirectory(directory)
                        writeNew(directory.resolve("owner"), (OWNER + key + "\n").toByteArray(Charsets.US_ASCII), written)
                        for (entry in entries) {
                            val target = directory.resolve(entry.name); requireContained(directory, target)
                            resources.open("win32-x86-64/${entry.name}")?.use { input ->
                                FileChannel.open(target, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { output ->
                                    written += CreatedFile(target, Files.readAttributes(target, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS).fileKey())
                                    val hash = MessageDigest.getInstance("SHA-256"); var count = 0L; val buffer = ByteArray(64 * 1024)
                                    while (true) {
                                        val n = input.read(buffer)
                                        if (n < 0) break
                                        if (n == 0) throw NativeRuntimeFailure("A native resource stream did not advance.")
                                        count = Math.addExact(count, n.toLong())
                                        if (count > entry.bytes) throw NativeRuntimeFailure("A native resource exceeds its declared size.")
                                        hash.update(buffer, 0, n)
                                        val bytes = ByteBuffer.wrap(buffer, 0, n); while (bytes.hasRemaining()) output.write(bytes)
                                    }
                                    if (count != entry.bytes || hex(hash.digest()) != entry.sha256) throw NativeRuntimeFailure("A packaged native resource failed its exact hash check.")
                                    output.force(true)
                                }
                            } ?: throw NativeRuntimeFailure("A packaged native runtime file is missing.")
                        }
                        writeNew(directory.resolve("ready"), manifest, written)
                    }
                }
                guard.check(directory)
                if (created == null) pinned += guard.pinDirectory(directory)
                if (directory.toRealPath() != directory) throw NativeRuntimeFailure("The native runtime path contains a redirected component.")
                val expectedNames = names + setOf("owner", "ready")
                val actual = Files.newDirectoryStream(directory).use { stream ->
                    val result = mutableSetOf<String>()
                    for (path in stream) { if (result.size >= expectedNames.size) throw NativeRuntimeFailure("The native runtime directory contains unknown entries."); result += path.fileName.toString() }
                    result
                }
                if (actual != expectedNames) throw NativeRuntimeFailure("The native runtime cache is incomplete. Existing files were preserved.")
                val owner = directory.resolve("owner"); val ready = directory.resolve("ready")
                pinned += guard.pin(owner); pinned += guard.pin(ready)
                if (!boundedRead(owner, 256).contentEquals((OWNER + key + "\n").toByteArray(Charsets.US_ASCII))
                    || !boundedRead(ready, 4096).contentEquals(manifest)) throw NativeRuntimeFailure("The native runtime ownership marker changed.")
                for (entry in entries) {
                    val file = directory.resolve(entry.name); guard.check(file)
                    pinned += guard.pin(file)
                    val info = Files.readAttributes(file, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
                    if (!info.isRegularFile || info.isSymbolicLink || info.isOther || info.size() != entry.bytes) throw NativeRuntimeFailure("The native runtime file layout changed.")
                    val hash = MessageDigest.getInstance("SHA-256")
                    Files.newInputStream(file).use { input ->
                        val buffer = ByteArray(64 * 1024); var count = 0L
                        while (true) { val n = input.read(buffer); if (n < 0) break; if (n == 0) throw NativeRuntimeFailure("A native cache read did not advance.")
                            count += n; if (count > entry.bytes) throw NativeRuntimeFailure("A native cache file grew during verification."); hash.update(buffer, 0, n) }
                        if (count != entry.bytes) throw NativeRuntimeFailure("A native cache file was truncated.")
                    }
                    if (hex(hash.digest()) != entry.sha256) throw NativeRuntimeFailure("A native cache file failed its exact hash check.")
                }
                return DesktopNativeRuntime(directory, pinned.toList(), entries.associate { it.name to it.sha256 })
            } catch (error: Exception) {
                pinned.asReversed().forEach { runCatching { it.close() } }
                // Only this call's newly-created, never-loaded files can be
                // cleaned. Preexisting caches and unexpected entries survive.
                created?.let { directory -> runCatching {
                    guard.check(directory)
                    if (directory.toRealPath() != directory) throw NativeRuntimeFailure("Native cleanup containment changed; files were preserved.")
                    for (file in written.asReversed()) {
                        requireContained(directory, file.path); guard.check(file.path)
                        val key = Files.readAttributes(file.path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS).fileKey()
                        if (file.key == null || key != file.key) throw NativeRuntimeFailure("A newly-created native file changed ownership; cleanup preserved it.")
                        Files.deleteIfExists(file.path)
                    }
                    Files.delete(directory) // nonrecursive; refuses new/unknown children
                } }
                if (error is NativeRuntimeFailure) throw error
                throw NativeRuntimeFailure("The private native runtime could not be prepared safely. Existing files were preserved.")
            }
        }
        private data class Entry(val sha256: String, val bytes: Long, val name: String)
        private data class CreatedFile(val path: Path, val key: Any?)
        private fun parseManifest(bytes: ByteArray): List<Entry> {
            if (bytes.size !in 1..4096 || bytes.any { it.toInt() !in 10..126 || it.toInt() in 11..31 }) throw NativeRuntimeFailure("The native runtime manifest is invalid.")
            val pattern = Regex("([0-9a-f]{64}) ([1-9][0-9]{0,8}) (vw_core\\.dll|vw_host\\.dll|vw-connection-helper\\.exe|vw-capture-helper\\.exe)")
            val entries = bytes.toString(Charsets.US_ASCII).lineSequence().filter(String::isNotEmpty).map { line ->
                val match = pattern.matchEntire(line) ?: throw NativeRuntimeFailure("The native runtime manifest is invalid.")
                val size = match.groupValues[2].toLong()
                if (size !in 1..MAX_FILE) throw NativeRuntimeFailure("A native runtime file exceeds its admission limit.")
                Entry(match.groupValues[1], size, match.groupValues[3])
            }.toList()
            if (entries.size != 4 || entries.map { it.name }.toSet() != names || entries.sumOf { it.bytes } > MAX_TOTAL) throw NativeRuntimeFailure("The native runtime inventory is incomplete or exceeds its bound.")
            return entries
        }
        private fun ensureDirectories(path: Path, guard: NativeFileGuard, pinned: MutableList<AutoCloseable>) {
            // Check and pin each existing ancestor before creating a child.
            // Otherwise a preexisting parent junction could receive a file
            // before a later containment check rejects it. Keeping the chain
            // pinned also prevents its rename during extraction and loading.
            val chain = generateSequence(path) { it.parent }.toList().asReversed()
            for (component in chain) {
                if (!Files.exists(component, LinkOption.NOFOLLOW_LINKS)) try { Files.createDirectory(component) } catch (_: FileAlreadyExistsException) { }
                guard.check(component)
                if (!Files.isDirectory(component, LinkOption.NOFOLLOW_LINKS)) throw NativeRuntimeFailure("A native runtime parent is not a directory.")
                pinned += guard.pinDirectory(component)
                if (component.toRealPath() != component) throw NativeRuntimeFailure("The native runtime path is redirected.")
            }
        }
        private fun boundedRead(path: Path, limit: Int): ByteArray = Files.newInputStream(path).use { it.readNBytes(limit + 1).also { bytes -> if (bytes.size > limit) throw NativeRuntimeFailure("A native runtime marker is oversized.") } }
        private fun writeNew(path: Path, bytes: ByteArray, written: MutableList<CreatedFile>) {
            FileChannel.open(path, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { file ->
                written += CreatedFile(path, Files.readAttributes(path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS).fileKey())
                val data = ByteBuffer.wrap(bytes); while (data.hasRemaining()) file.write(data); file.force(true)
            }
        }
        private fun requireContained(root: Path, child: Path) {
            if (child.toAbsolutePath().normalize() != child || child.parent != root) throw NativeRuntimeFailure("Native runtime path containment failed.")
        }
        private fun digest(bytes: ByteArray) = hex(MessageDigest.getInstance("SHA-256").digest(bytes))
        private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it.toInt() and 255) }
    }
}

/** JNA core is already an approved dependency. Loading the system kernel32
 * library performs no host-binding or AWT initialization. */
internal interface NativeRuntimeKernel : StdCallLibrary {
        fun GetFileAttributesW(path: WString): Int
        fun CreateFileW(path: WString, access: Int, share: Int, security: Pointer?, disposition: Int, flags: Int, template: Pointer?): Pointer?
        fun GetFileInformationByHandleEx(handle: Pointer, kind: Int, buffer: Pointer, bytes: Int): Boolean
        fun GetFinalPathNameByHandleW(handle: Pointer, buffer: CharArray, count: Int, flags: Int): Int
        fun CloseHandle(handle: Pointer): Boolean
}
internal class WindowsNativeFileGuard : NativeAnchorGuard {
    private val kernel: NativeRuntimeKernel = Native.load("kernel32", NativeRuntimeKernel::class.java)
    override fun check(path: Path) {
        val flags = kernel.GetFileAttributesW(WString(nativeGuardPath(path)))
        if (flags == -1 || flags and 0x400 != 0) throw NativeRuntimeFailure("A native runtime path is missing or is a reparse point.")
    }
    override fun pin(path: Path): AutoCloseable {
        // GENERIC_READ, FILE_SHARE_READ only, OPEN_EXISTING. Denies writes and
        // rename/delete while the helper and DLL hashes remain authoritative.
        return openPin(path, Int.MIN_VALUE, 1, 0x80, directory = false).lease
    }
    override fun pinDirectory(path: Path): AutoCloseable =
        // LIST_DIRECTORY participates in Windows sharing checks. An
        // attributes-only handle does not prevent directory rename.
        openPin(path, 0x81, 3, 0x02000000, directory = true).lease
    override fun pinAnchor(path: Path): NativeDirectoryLease =
        openPin(path, 0x81, 3, 0x02000000, directory = true, canonicalAnchor = true)
    private fun openPin(path: Path, access: Int, share: Int, flags: Int,
                        directory: Boolean, canonicalAnchor: Boolean = false): NativeDirectoryLease {
        check(path)
        // OPEN_REPARSE_POINT means a racing junction is opened as the link,
        // then refused by the handle-based attribute check, never followed.
        val handle = kernel.CreateFileW(WString(nativeGuardPath(path)), access, share, null, 3, flags or 0x00200000, null)
        if (handle == null || Pointer.nativeValue(handle) == -1L) throw NativeRuntimeFailure("A native runtime file could not be pinned against replacement.")
        val canonical: Path
        try {
            Memory(8).use { attributes ->
                if (!kernel.GetFileInformationByHandleEx(handle, 9, attributes, 8) || attributes.getInt(0) and 0x400 != 0)
                    throw NativeRuntimeFailure("A native runtime handle is a reparse point.")
                if ((attributes.getInt(0) and 0x10 != 0) != directory)
                    throw NativeRuntimeFailure("A native runtime handle has an unexpected file type.")
            }
            val resolved = CharArray(32768)
            val length = kernel.GetFinalPathNameByHandleW(handle, resolved, resolved.size, 0)
            if (length <= 0 || length >= resolved.size)
                throw NativeRuntimeFailure("A native runtime handle returned an invalid path length.")
            canonical = nativeCanonicalDosPath(String(resolved, 0, length))
            if (!canonicalAnchor && canonical != path)
                throw NativeRuntimeFailure("A native runtime handle resolved outside its expected path.")
        } catch (error: Exception) { kernel.CloseHandle(handle); throw error }
        val closed = AtomicBoolean(false)
        return NativeDirectoryLease(canonical, AutoCloseable {
            if (closed.compareAndSet(false, true)) kernel.CloseHandle(handle)
        }, {
            if (closed.get()) throw NativeRuntimeFailure("The native cache anchor lease has closed.")
            Memory(8).use { attributes ->
                if (!kernel.GetFileInformationByHandleEx(handle, 9, attributes, 8) ||
                    attributes.getInt(0) and 0x400 != 0 || (attributes.getInt(0) and 0x10 != 0) != directory)
                    throw NativeRuntimeFailure("The native cache anchor handle changed its file type.")
            }
            val resolved = CharArray(32768)
            val length = kernel.GetFinalPathNameByHandleW(handle, resolved, resolved.size, 0)
            if (length <= 0 || length >= resolved.size ||
                nativeCanonicalDosPath(String(resolved, 0, length)) != canonical)
                throw NativeRuntimeFailure("The native cache anchor changed during physical path pinning.")
        })
    }
}
