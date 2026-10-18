package com.visualworkbench.shared

import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest
import java.util.Base64
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

/** Actual optional-capability callers; execute after the single final ABI/native
 * build. Synthetic local originals only; no platform decoder/runtime approval. */
class GiantImportNativeCallerTest {
    private class Input : AutoCloseable {
        val root: Path = Files.createTempDirectory("vw-public-giant-caller-")
        val source: Path = root.resolve("selected.png")
        val work: Path = Files.createDirectory(root.resolve("work"))
        val bytes: ByteArray = Base64.getDecoder().decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQ1QoBAADrAJRe4xMbAAAAAElFTkSuQmCC")
        init { Files.write(source, bytes) }
        fun options(): GiantOriginalOptions = GiantOriginalOptions(source.toString(), work.toString(),
            4uL * 1024uL * 1024uL, 512uL * 1024uL * 1024uL, true)
        fun children(): Long = Files.list(work).use { it.count() }
        override fun close() {
            check(children() == 0L) { "Keep synthetic fixture while a native input owner remains" }
            Files.delete(work); Files.delete(source); Files.delete(root)
        }
    }
    @Test fun actualPrepareKeepsIndependentPreparedOwnerAndPreservesOriginal() = runBlocking {
        val core = workbenchCore() as WorkbenchGiantImportCore
        Input().use { input ->
            val picked = core.openGiantOriginal(input.options())
            val prepared = picked.prepare()
            picked.close()
            try {
                val info = prepared.describe()
                assertEquals(1u, info.width); assertEquals(1u, info.height)
                assertEquals("png", info.format); assertEquals(input.bytes.size.toULong(), info.encodedBytes)
                assertTrue(info.assetId.matches(Regex("[0-9a-f]{64}")))
                val digest = MessageDigest.getInstance("SHA-256").digest(input.bytes).joinToString("") { "%02x".format(it.toInt() and 255) }
                assertEquals(digest, info.sha256)
                assertEquals(GiantImageFailureKind.Unsupported,
                    (runCatching { prepared.build(GiantCacheBuildOptions(32uL * 1024uL * 1024uL, 1000u, false)) }.exceptionOrNull() as GiantImageFailure).kind)
                assertEquals(GiantImageFailureKind.Limit,
                    (runCatching { prepared.build(GiantCacheBuildOptions(0u, 1000u, true)) }.exceptionOrNull() as GiantImageFailure).kind)
                assertEquals(1L, input.children())
            } finally { prepared.close() }
            assertEquals(0L, input.children()); assertArrayEquals(input.bytes, Files.readAllBytes(input.source))
            assertEquals(GiantImageFailureKind.Closed,
                (runCatching { prepared.describe() }.exceptionOrNull() as GiantImageFailure).kind)
        }
    }
    @Test fun thirdCallerIsRefusedBeforeAccessingMissingPaths() = runBlocking {
        val core = workbenchCore() as WorkbenchGiantImportCore
        Input().use { input ->
            val first = core.openGiantOriginal(input.options())
            val second = core.openGiantOriginal(input.options())
            try {
                val options = GiantOriginalOptions(input.source.toString() + ".missing",
                    input.work.toString() + ".missing", 1024u, 1024u, true)
                assertEquals(GiantImageFailureKind.Backpressure,
                    (runCatching { core.openGiantOriginal(options) }.exceptionOrNull() as GiantImageFailure).kind)
                assertEquals(2L, input.children())
            } finally { first.close(); second.close() }
            assertEquals(0L, input.children())
        }
    }
    @Test fun uriAndUnconfirmedInterpretationAreTypedRefusals() = runBlocking {
        val core = workbenchCore() as WorkbenchGiantImportCore
        Input().use { input ->
            assertEquals(GiantImageFailureKind.Invalid,
                (runCatching { core.openGiantOriginal(GiantOriginalOptions("content://synthetic/original",
                    input.work.toString(), 1024u, 1024u, true)) }.exceptionOrNull() as GiantImageFailure).kind)
            assertEquals(GiantImageFailureKind.Unsupported,
                (runCatching { core.openGiantOriginal(GiantOriginalOptions(input.source.toString(),
                    input.work.toString(), 1024u, 1024u, false)) }.exceptionOrNull() as GiantImageFailure).kind)
            assertEquals(0L, input.children())
        }
    }
}
