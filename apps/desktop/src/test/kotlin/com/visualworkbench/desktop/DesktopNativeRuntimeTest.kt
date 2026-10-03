package com.visualworkbench.desktop

import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.ByteArrayInputStream
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean

class DesktopNativeRuntimeTest {
    @get:Rule val temporary = TemporaryFolder()
    @Test fun exactRuntimeIsPreparedBeforeAnyLoadAndClosePreservesLoadedFilePaths() {
        val files = runtimeResources(); val guard = MemoryFileGuard()
        val runtime = DesktopNativeRuntime.prepareAt(root(), files, guard)
        assertEquals(root().nameCount + 7, guard.live) // ancestors including volume root, cache, and five exact files
        for (name in runtimeNames) assertArrayEquals(files.bytes.getValue("win32-x86-64/$name"), Files.readAllBytes(runtime.directory.resolve(name)))
        assertTrue(Files.isRegularFile(runtime.directory.resolve("owner")))
        assertTrue(Files.isRegularFile(runtime.directory.resolve("ready")))
        runtime.close(); runtime.close(); assertEquals(0, guard.live)
        assertTrue(Files.isDirectory(runtime.directory)); assertTrue(Files.isRegularFile(runtime.directory.resolve("vw_host.dll")))
    }
    @Test fun anIdenticalBuildReusesOnlyTheCompleteHashBoundOwnedDirectory() {
        val files = runtimeResources(); val first = DesktopNativeRuntime.prepareAt(root(), files, MemoryFileGuard())
        val before = Files.readAttributes(first.directory.resolve("vw_host.dll"), BasicFileAttributes::class.java).fileKey()
        val next = DesktopNativeRuntime.prepareAt(root(), files, MemoryFileGuard())
        try { assertEquals(first.directory, next.directory); assertEquals(before, Files.readAttributes(next.directory.resolve("vw_host.dll"), BasicFileAttributes::class.java).fileKey()) }
        finally { next.close(); first.close() }
    }
    @Test fun changedExistingBytesAreRefusedWithoutOverwriteOrDeletion() {
        val files = runtimeResources(); val original = DesktopNativeRuntime.prepareAt(root(), files, MemoryFileGuard()); original.close()
        val file = original.directory.resolve("vw_host.dll"); val changed = "owner changed existing bytes".toByteArray(); Files.write(file, changed)
        assertFailure { DesktopNativeRuntime.prepareAt(root(), files, MemoryFileGuard()) }
        assertArrayEquals(changed, Files.readAllBytes(file)); assertTrue(Files.isRegularFile(original.directory.resolve("ready")))
    }
    @Test fun unknownEntriesInAnExistingCacheSurviveRefusal() {
        val files = runtimeResources(); val original = DesktopNativeRuntime.prepareAt(root(), files, MemoryFileGuard()); original.close()
        val unrelated = original.directory.resolve("unrelated.txt"); Files.writeString(unrelated, "preserve")
        assertFailure { DesktopNativeRuntime.prepareAt(root(), files, MemoryFileGuard()) }
        assertEquals("preserve", Files.readString(unrelated)); assertTrue(Files.exists(original.directory.resolve("vw_core.dll")))
    }
    @Test fun malformedDuplicateMissingAndOversizedInventoriesFailBeforeExtraction() {
        val files = runtimeResources(); val manifest = files.bytes.getValue("vw-native-runtime.sha256").toString(Charsets.US_ASCII)
        for (invalid in listOf("x".repeat(4097), manifest + manifest.lineSequence().first() + "\n", manifest.lines().drop(1).joinToString("\n"), manifest.replace("vw_host.dll", "../vw_host.dll"), manifest.replace(" 12 ", " 999999999 "))) {
            val resource = MapNativeResources(files.bytes.toMutableMap().also { it["vw-native-runtime.sha256"] = invalid.toByteArray() })
            val parent = temporary.newFolder().toPath().resolve("not-created")
            assertFailure { DesktopNativeRuntime.prepareAt(parent, resource, MemoryFileGuard()) }
            assertFalse(Files.exists(parent))
        }
    }
    @Test fun hashOrLengthMismatchNeverPublishesAReadyRuntime() {
        for (payload in listOf("wrong bytes!".toByteArray(), "short".toByteArray(), ByteArray(13))) {
            val files = runtimeResources(); files.bytes["win32-x86-64/vw_core.dll"] = payload
            val parent = temporary.newFolder().toPath().resolve("runtime")
            assertFailure { DesktopNativeRuntime.prepareAt(parent, files, MemoryFileGuard()) }
            Files.walk(parent).use { paths -> assertFalse(paths.anyMatch { it.fileName.toString() == "ready" }) }
        }
    }
    @Test fun injectedReparseRefusalReleasesEveryAcquiredLeaseAndPreservesPriorCache() {
        val files = runtimeResources(); val original = DesktopNativeRuntime.prepareAt(root(), files, MemoryFileGuard()); original.close()
        val guard = MemoryFileGuard { it.fileName?.toString() == "vw-connection-helper.exe" }
        assertFailure { DesktopNativeRuntime.prepareAt(root(), files, guard) }
        assertEquals(0, guard.live); assertTrue(Files.isRegularFile(original.directory.resolve("vw-connection-helper.exe")))
    }
    @Test fun parentReparseIsRefusedBeforeCreatingAnyChild() {
        val ancestor = temporary.newFolder().toPath().toAbsolutePath().normalize()
        val child = ancestor.resolve("not-created/runtime")
        val guard = MemoryFileGuard { it == ancestor }
        assertFailure { DesktopNativeRuntime.prepareAt(child, runtimeResources(), guard) }
        assertEquals(0, guard.live); assertFalse(Files.exists(ancestor.resolve("not-created")))
    }
    private fun root(): Path = temporary.root.toPath().toAbsolutePath().normalize().resolve("runtime")
    private fun assertFailure(block: () -> Unit) { try { block(); fail("unverified runtime accepted") } catch (_: NativeRuntimeFailure) { } }
}
private val runtimeNames = listOf("vw_core.dll", "vw_host.dll", "vw-connection-helper.exe")
private class MapNativeResources(val bytes: MutableMap<String, ByteArray>) : NativeResources {
    override fun open(name: String) = bytes[name]?.let(::ByteArrayInputStream)
}
private fun runtimeResources(): MapNativeResources {
    val files = runtimeNames.associate { "win32-x86-64/$it" to "MZtest-bytes".padEnd(12, '_').toByteArray() }.toMutableMap()
    val manifest = runtimeNames.sorted().joinToString("\n", postfix = "\n") { name ->
        val data = files.getValue("win32-x86-64/$name"); val hash = MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it.toInt() and 255) }
        "$hash ${data.size} $name"
    }
    files["vw-native-runtime.sha256"] = manifest.toByteArray(Charsets.US_ASCII)
    return MapNativeResources(files)
}
private class MemoryFileGuard(private val reject: (Path) -> Boolean = { false }) : NativeFileGuard {
    var live = 0
    override fun check(path: Path) {
        val info = Files.readAttributes(path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
        if (reject(path) || info.isOther || info.isSymbolicLink) throw NativeRuntimeFailure("Injected reparse refusal")
    }
    override fun pin(path: Path): AutoCloseable { check(path); live++; val closed = AtomicBoolean(false); return AutoCloseable { if (closed.compareAndSet(false, true)) live-- } }
    override fun pinDirectory(path: Path) = pin(path)
}
