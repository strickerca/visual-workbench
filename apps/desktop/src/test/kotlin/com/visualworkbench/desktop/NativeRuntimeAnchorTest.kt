package com.visualworkbench.desktop

import org.junit.Assert.*
import org.junit.Assume.assumeTrue
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

/** Synthetic resources and task-owned directories only. No DLL is loaded, no
 * AWT window is opened, and no owner app-data path is read by these tests. */
class NativeRuntimeAnchorTest {
    @get:Rule val temporary = TemporaryFolder()

    @Test fun redirectedAnchorUsesCanonicalChildrenAndRetainsBothLeaseChains() {
        val root = local()
        val physical = root.resolve("Packages/synthetic/LocalCache/Local/Visual Workbench")
        Files.createDirectories(physical)
        val guard = AnchorGuard(resolve = { physical })
        val resources = resources { name ->
            if (name.startsWith("win32-x86-64/")) {
                // File extraction is admitted only after every canonical
                // ancestor has its ordinary, exact-path directory pin.
                for (part in generateSequence(physical) { it.parent })
                    assertTrue("physical ancestor not pinned", part in guard.directories)
                assertTrue(guard.anchorLive)
            }
        }
        val runtime = DesktopNativeRuntime.prepareUnderAppData(root, resources, guard)
        try {
            assertEquals(physical.resolve("native-runtime"), runtime.directory.parent)
            assertTrue(root in guard.directories)
            assertTrue(guard.anchorLive)
            Files.newDirectoryStream(root.resolve("Visual Workbench")).use { assertFalse(it.iterator().hasNext()) }
            assertTrue(Files.isRegularFile(runtime.directory.resolve("ready")))
        } finally { runtime.close(); runtime.close() }
        assertEquals(0, guard.live)
        assertFalse(guard.anchorLive)
        assertTrue(Files.isRegularFile(runtime.directory.resolve("vw_core.dll")))
    }

    @Test fun ordinaryUnredirectedAnchorReusesExactlyTheSameHashBoundCache() {
        val root = local(); val firstGuard = AnchorGuard()
        val first = DesktopNativeRuntime.prepareUnderAppData(root, resources(), firstGuard)
        val secondGuard = AnchorGuard()
        val second = DesktopNativeRuntime.prepareUnderAppData(root, resources(), secondGuard)
        try { assertEquals(first.directory, second.directory) }
        finally { second.close(); first.close() }
        assertEquals(0, firstGuard.live); assertEquals(0, secondGuard.live)
    }

    @Test fun missingOriginalAppDataIsNeverCreated() {
        val root = temporary.root.toPath().toAbsolutePath().resolve("missing")
        val guard = AnchorGuard()
        failure { DesktopNativeRuntime.prepareUnderAppData(root, resources(), guard) }
        assertEquals(0, guard.live); assertFalse(Files.exists(root))
    }

    @Test fun originalAncestorRefusalPrecedesEvenAnchorCreation() {
        val root = local(); val guard = AnchorGuard(reject = { it == root.parent })
        failure { DesktopNativeRuntime.prepareUnderAppData(root, resources(), guard) }
        assertFalse(Files.exists(root.resolve("Visual Workbench"))); assertEquals(0, guard.live)
    }

    @Test fun anchorReparseRefusalNeverCreatesRuntimeChildren() {
        val root = local()
        val guard = AnchorGuard(anchorRefusal = true)
        failure { DesktopNativeRuntime.prepareUnderAppData(root, resources(), guard) }
        Files.newDirectoryStream(root.resolve("Visual Workbench")).use { assertFalse(it.iterator().hasNext()) }
        assertEquals(0, guard.live)
    }

    @Test fun outsideRootRootItselfWrongLeafRelativeAndDotPathsAreRefused() {
        val root = local()
        val outside = temporary.newFolder("outside").toPath().toAbsolutePath().resolve("Visual Workbench")
        Files.createDirectory(outside); Files.writeString(outside.resolve("unrelated"), "preserve")
        val invalid = listOf(outside, root, root.resolve("Other"), Path.of("Visual Workbench"),
            root.resolve("child/../Visual Workbench"))
        for (path in invalid) {
            val guard = AnchorGuard(resolve = { path })
            failure { DesktopNativeRuntime.prepareUnderAppData(root, resources(), guard) }
            assertEquals(0, guard.live); assertFalse(guard.anchorLive)
        }
        assertEquals("preserve", Files.readString(outside.resolve("unrelated")))
        assertFalse(Files.exists(outside.resolve("native-runtime")))
    }

    @Test fun physicalAncestorRefusalPrecedesAnyExtractionAndReleasesOriginalPins() {
        val root = local()
        val packages = root.resolve("Packages")
        val physical = packages.resolve("synthetic/LocalCache/Local/Visual Workbench")
        Files.createDirectories(physical)
        val guard = AnchorGuard(resolve = { physical }, reject = { it == packages })
        var payloadReads = 0
        failure { DesktopNativeRuntime.prepareUnderAppData(root,
            resources { if (it.startsWith("win32-x86-64/")) payloadReads++ }, guard) }
        assertEquals(0, payloadReads); assertEquals(0, guard.live)
        assertFalse(Files.exists(physical.resolve("native-runtime")))
    }

    @Test fun verificationFailureReleasesAnchorAndPreservesUnknownPhysicalFiles() {
        val root = local(); val guard = AnchorGuard()
        val first = DesktopNativeRuntime.prepareUnderAppData(root, resources(), guard)
        val directory = first.directory; first.close()
        Files.writeString(directory.resolve("unknown"), "preserve")
        val next = AnchorGuard()
        failure { DesktopNativeRuntime.prepareUnderAppData(root, resources(), next) }
        assertEquals(0, next.live); assertEquals("preserve", Files.readString(directory.resolve("unknown")))
        assertTrue(Files.isRegularFile(directory.resolve("ready")))
    }

    @Test fun anchorMovingDuringPhysicalHandoffRefusesBeforeRuntimeActivation() {
        val root = local(); val guard = AnchorGuard(recheckRefusal = true)
        failure { DesktopNativeRuntime.prepareUnderAppData(root, resources(), guard) }
        assertEquals(0, guard.live); assertFalse(guard.anchorLive)
        // Complete, never-loaded hash-bound bytes are preserved for inspection;
        // a bootstrap failure never deletes another caller's cache.
        assertTrue(Files.isDirectory(root.resolve("Visual Workbench/native-runtime")))
    }

    @Test fun boundedDosParserRefusesOtherNamespacesAndNoncanonicalForms() {
        windows()
        assertEquals(Path.of("C:\\synthetic\\Visual Workbench"),
            nativeCanonicalDosPath("\\\\?\\C:\\synthetic\\Visual Workbench"))
        val invalid = listOf("C:\\synthetic", "\\\\?\\C:relative", "\\\\?\\UNC\\server\\share",
            "\\\\.\\C:\\synthetic", "\\\\?\\Volume{fake}\\synthetic", "\\\\?\\C:\\a\\..\\b",
            "\\\\?\\C:\\a\\.\\b", "\\\\?\\C:\\a:stream", "\\\\?\\C:/synthetic", "\\\\?\\C:\\" + "x".repeat(32768))
        for (value in invalid) failure { nativeCanonicalDosPath(value) }
    }

    @Test fun realWindowsAnchorHandleIsCanonicalAndOrdinaryPinsRemainExactAndExclusive() {
        windows()
        val owned = temporary.newFolder("real-guard").toPath().toAbsolutePath().normalize()
        val guard = WindowsNativeFileGuard()
        val anchor = guard.pinAnchor(owned)
        try {
            assertEquals(owned.toRealPath(), anchor.path)
            val physical = anchor.path
            ioFailure { Files.move(physical, physical.resolveSibling("must-not-move")) }
            Files.createDirectory(physical.resolve("child"))
            guard.pinDirectory(physical.resolve("child")).use {
                ioFailure { Files.move(physical.resolve("child"), physical.resolve("must-not-move-child")) }
            }
            // Only pinAnchor may return a canonical path. Ordinary pins must
            // reject an opened spelling whose normalized handle path differs.
            failure { guard.pinDirectory(physical.resolve("child/..")).close() }
            val file = physical.resolve("owned.bin"); Files.write(file, byteArrayOf(1, 2, 3))
            guard.pin(file).use {
                ioFailure { Files.write(file, byteArrayOf(4)) }
                ioFailure { Files.delete(file) }
            }
            assertArrayEquals(byteArrayOf(1, 2, 3), Files.readAllBytes(file))
            failure { guard.pinAnchor(file).lease.close() }
        } finally { anchor.lease.close() }
    }

    @Test fun longNativeGuardPathsAreExplicitWithoutChangingTheShortAnchorNamespace() {
        windows()
        val short = Path.of("C:\\synthetic\\Visual Workbench")
        assertEquals(short.toString(), nativeGuardPath(short))
        val long = short.resolve("x".repeat(120)).resolve("y".repeat(120)).resolve("owned.bin")
        assertTrue(long.toString().length >= 260)
        assertEquals("\\\\?\\" + long, nativeGuardPath(long))
        failure { nativeGuardPath(long.resolve("..")) }
        failure { nativeGuardPath(Path.of("relative").resolve("x".repeat(260))) }
    }

    @Test fun realLongWindowsPinsStillRefuseWritesAndReplacement() {
        windows()
        val root = temporary.newFolder("long-guard").toPath().toRealPath()
        val directory = Files.createDirectories(root.resolve("x".repeat(120)).resolve("y".repeat(120)))
        val file = directory.resolve("owned.bin")
        assertTrue(file.toString().length >= 260)
        Files.write(file, byteArrayOf(1, 2, 3))
        val guard = WindowsNativeFileGuard()
        try {
            guard.check(directory); guard.check(file)
            guard.pinDirectory(directory).use {
                guard.pin(file).use {
                    assertArrayEquals(byteArrayOf(1, 2, 3), Files.readAllBytes(file))
                    ioFailure { Files.write(file, byteArrayOf(4)) }
                    ioFailure { Files.delete(file) }
                    ioFailure { Files.move(directory, directory.resolveSibling("replacement")) }
                }
            }
            Files.write(file, byteArrayOf(4))
            assertArrayEquals(byteArrayOf(4), Files.readAllBytes(file))
        } finally {
            // JUnit's java.io.File cleanup may itself use legacy path limits.
            Files.walk(root).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach { Files.delete(it) } }
        }
    }

    private fun local(): Path = temporary.newFolder().toPath().toAbsolutePath().normalize()
    private fun windows() { assumeTrue(System.getProperty("os.name").startsWith("Windows")) }
    private fun failure(block: () -> Any?) { try { block(); fail("Unsafe cache path was accepted") } catch (_: NativeRuntimeFailure) { } }
    private fun ioFailure(block: () -> Any?) { try { block(); fail("Pinned native path was mutable") } catch (_: java.io.IOException) { } }

    private class AnchorGuard(
        val resolve: (Path) -> Path = { it },
        val reject: (Path) -> Boolean = { false },
        val anchorRefusal: Boolean = false,
        val recheckRefusal: Boolean = false,
    ) : NativeAnchorGuard {
        var live = 0
        var anchorLive = false
        val directories = mutableSetOf<Path>()
        override fun check(path: Path) {
            val info = Files.readAttributes(path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
            if (reject(path) || info.isOther || info.isSymbolicLink) throw NativeRuntimeFailure("Injected reparse refusal")
        }
        private fun lease(anchor: Boolean = false): AutoCloseable {
            live++; if (anchor) anchorLive = true
            val closed = AtomicBoolean(false)
            return AutoCloseable { if (closed.compareAndSet(false, true)) { live--; if (anchor) anchorLive = false } }
        }
        override fun pin(path: Path): AutoCloseable { check(path); return lease() }
        override fun pinDirectory(path: Path): AutoCloseable { check(path); directories.add(path); return lease() }
        override fun pinAnchor(path: Path): NativeDirectoryLease {
            check(path)
            if (anchorRefusal) throw NativeRuntimeFailure("Injected handle reparse refusal")
            return NativeDirectoryLease(resolve(path), lease(anchor = true)) {
                if (!anchorLive || recheckRefusal) throw NativeRuntimeFailure("Injected handle handoff refusal")
            }
        }
    }
    private fun resources(opened: (String) -> Unit = {}): NativeResources {
        val names = listOf("vw_core.dll", "vw_host.dll", "vw-connection-helper.exe", "vw-capture-helper.exe", "vw-hevc-helper.exe", "vw-input-helper.exe")
        val bytes = names.associate { "win32-x86-64/$it" to "Synthetic unexecuted bytes".toByteArray() }.toMutableMap()
        bytes["vw-native-runtime.sha256"] = names.joinToString("\n", postfix = "\n") { name ->
            val data = bytes.getValue("win32-x86-64/$name")
            val hash = MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it.toInt() and 255) }
            "$hash ${data.size} $name"
        }.toByteArray(Charsets.US_ASCII)
        return NativeResources { name -> opened(name); bytes[name]?.let(::ByteArrayInputStream) }
    }
}
