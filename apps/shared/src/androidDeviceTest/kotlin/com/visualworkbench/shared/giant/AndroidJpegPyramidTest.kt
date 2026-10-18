package com.visualworkbench.shared.giant

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.ColorSpace
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.shared.SessionResourceCharge
import com.visualworkbench.shared.SessionResources
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.yield
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File
import java.io.FileOutputStream
import java.nio.file.Files
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Deterministic device cases, written source only in the delegated phase.
 * Fixture allocations are deliberately isolated from production admission. */
public class AndroidJpegPyramidTest {
    private class Ledger(private val limit: Long = AndroidJpegPyramid.SESSION_ENVELOPE_BYTES) : SessionResources {
        private val gate = Any()
        var used: Long = 0
            private set
        var calls: Int = 0
            private set
        var releases: Int = 0
            private set
        var failRelease: Boolean = false
        override suspend fun reserveForeign(bytes: ULong): SessionResourceCharge = synchronized(gate) {
            calls++
            if (bytes > limit.toULong() || used > limit - bytes.toLong()) throw TileBuildFailure("session budget")
            used += bytes.toLong()
            object : SessionResourceCharge {
                private var closed = false
                override fun close(): Unit = synchronized(gate) {
                    if (!closed) {
                        if (failRelease) throw IllegalStateException("fixture release failure")
                        used -= bytes.toLong(); releases++; closed = true
                    }
                }
            }
        }
    }

    private fun root(): File = File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.canonicalFile,
        "giant-test-${UUID.randomUUID()}").also { check(it.mkdir()) }
    private fun work(root: File): File = File(root, "work").also { check(it.mkdir()) }
    private fun sha(file: File): String = MessageDigest.getInstance("SHA-256").digest(file.readBytes())
        .joinToString("") { "%02x".format(it.toInt() and 255) }
    private fun jpeg(root: File, width: Int = 513, height: Int = 257, orientation: Int = 1): File {
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        try {
            val pixels = IntArray(width * height) { i -> Color.rgb(i % 251, (i / width) % 241, (i / 7) % 239) }
            bitmap.setPixels(pixels, 0, width, 0, 0, width, height)
            return File(root, "source-$orientation.jpg").also { file ->
                FileOutputStream(file).use { check(bitmap.compress(Bitmap.CompressFormat.JPEG, 95, it)); it.fd.sync() }
                if (orientation != 1) {
                    val bytes = file.readBytes()
                    val exif = byteArrayOf(69, 120, 105, 102, 0, 0, 73, 73, 42, 0, 8, 0, 0, 0,
                        1, 0, 18, 1, 3, 0, 1, 0, 0, 0, orientation.toByte(), 0, 0, 0, 0, 0, 0, 0)
                    file.writeBytes(bytes.copyOfRange(0, 2) + byteArrayOf(0xff.toByte(), 0xe1.toByte(), 0, 34) + exif + bytes.copyOfRange(2, bytes.size))
                }
            }
        } finally { bitmap.recycle() }
    }
    private fun input(file: File, work: File, width: Int = 513, height: Int = 257, orientation: Int = 1,
        digest: String = sha(file), consent: Boolean = true, disk: Long = 3L * 1024 * 1024 * 1024): JpegTileInput =
        JpegTileInput.fromRootPrepared(file, work, digest, width, height, orientation, file.length(), disk, consent)
    private suspend fun refuse(block: suspend () -> Unit): TileBuildFailure {
        try { block() } catch (error: TileBuildFailure) { return error }
        throw AssertionError("expected refusal")
    }
    private fun dispose(root: File, work: File, vararg originals: File): Unit {
        for (file in originals) if (file.exists()) check(file.delete())
        check(work.listFiles()?.isEmpty() == true)
        check(work.delete()); check(root.delete())
    }

    @Test public fun rawBandInverseCoversExactSourceForAllEightOrientations(): Unit {
        for (orientation in 1..8) {
            val width = if (orientation >= 5) 257 else 513
            val height = if (orientation >= 5) 513 else 257
            var area = 0L
            for (y in 0 until height step 256) {
                val rect = AndroidJpegPyramid.rawRect(0, y, width, minOf(256, height - y), 513, 257, orientation)
                assertTrue(rect.left >= 0 && rect.right <= 513 && rect.top >= 0 && rect.bottom <= 257)
                area += rect.width().toLong() * rect.height()
            }
            assertEquals(513L * 257, area)
        }
    }

    @Test public fun sharedPoolRefusesBeforeSourceOrCodecAccessAndReleasesSlot(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root)
        val ledger = Ledger()
        val otherOwner = ledger.reserveForeign(1uL)
        val prepared = input(file, work)
        check(file.delete()) // Budget failure must precede even source opening.
        try {
            assertEquals("session budget", refuse { AndroidJpegPyramid.build(prepared, ledger) { fail("codec entered") } }.reason)
            assertEquals(1L, ledger.used)
            assertTrue(work.listFiles()!!.isEmpty())
        } finally { otherOwner.close() }
        val retry = jpeg(root)
        try {
            val result = AndroidJpegPyramid.build(input(retry, work), ledger) { }
            try {
                assertEquals(AndroidJpegPyramid.SESSION_ENVELOPE_BYTES, ledger.used)
                assertEquals("session budget", refuse { ledger.reserveForeign(1uL) }.reason)
            } finally { result.close() }
            assertEquals(0L, ledger.used)
        } finally { dispose(root, work, retry) }
    }

    @Test public fun actualLosslessBaseMatchesDecodedJpegForAllEightOddEdgeOrientations(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val ledger = Ledger()
        try {
            for (orientation in 1..8) {
                val file = jpeg(root, orientation = orientation)
                val before = file.readBytes()
                try {
                    val result = AndroidJpegPyramid.build(input(file, work, orientation = orientation), ledger) { }
                    try {
                        assertEquals(AndroidJpegPyramid.SESSION_ENVELOPE_BYTES, ledger.used)
                        val raw = checkNotNull(BitmapFactory.decodeFile(file.path))
                        val expected = AndroidJpegPyramid.orient(raw, orientation)
                        try {
                            assertEquals(expected.width, result.width); assertEquals(expected.height, result.height)
                            for (y in 0 until result.height step 256) for (x in 0 until result.width step 256) {
                                val tile = checkNotNull(BitmapFactory.decodeFile(File(result.directory, "L0/${x / 256}_${y / 256}.webp").path))
                                try {
                                    val actual = IntArray(tile.width * tile.height); val wanted = IntArray(actual.size)
                                    tile.getPixels(actual, 0, tile.width, 0, 0, tile.width, tile.height)
                                    expected.getPixels(wanted, 0, tile.width, x, y, tile.width, tile.height)
                                    assertArrayEquals(wanted, actual)
                                    assertTrue(File(result.directory, "L0/${x / 256}_${y / 256}.webp").length() <= AndroidJpegPyramid.MAX_TILE_BYTES)
                                } finally { tile.recycle() }
                            }
                            assertFalse(result.directory.listFiles()!!.any { it.name.startsWith("R") })
                            val last = result.directory.listFiles()!!.filter { it.isDirectory }.maxBy { it.name.drop(1).toInt() }
                            val smallest = checkNotNull(BitmapFactory.decodeFile(File(last, "0_0.jpg").path))
                            try { assertEquals(1, smallest.width); assertEquals(1, smallest.height) } finally { smallest.recycle() }
                        } finally { if (expected !== raw) expected.recycle(); raw.recycle() }
                    } finally { result.close() }
                    assertEquals(0L, ledger.used)
                    assertArrayEquals(before, file.readBytes())
                } finally { check(file.delete()) }
            }
        } finally { dispose(root, work) }
    }

    @Test public fun cancellationRetainsActualWorkerChargeAndSlotThroughCodecSettlement(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        val entered = CountDownLatch(1); val release = CountDownLatch(1)
        val operation = async(Dispatchers.Default) {
            AndroidJpegPyramid.build(input(file, work), ledger) { progress ->
                if (progress.phase == TileBuildPhase.Preview) { entered.countDown(); check(release.await(10, TimeUnit.SECONDS)) }
            }
        }
        try {
            assertTrue(withContext(Dispatchers.IO) { entered.await(10, TimeUnit.SECONDS) })
            operation.cancel(); yield()
            assertFalse(operation.isCompleted)
            assertEquals(AndroidJpegPyramid.SESSION_ENVELOPE_BYTES, ledger.used)
            assertEquals("busy", refuse { AndroidJpegPyramid.build(input(file, work), ledger) { } }.reason)
        } finally {
            release.countDown()
            withContext(NonCancellable) { try { operation.await() } catch (_: CancellationException) { } }
        }
        try { assertEquals(0L, ledger.used); assertTrue(work.listFiles()!!.isEmpty()) }
        finally { dispose(root, work, file) }
    }

    @Test public fun lateCompletedWorkerIsCleanedAfterCancelledAwaitWithoutPublishingReceipt(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        val entered = CountDownLatch(1); val release = CountDownLatch(1)
        val operation = async(Dispatchers.Default) {
            AndroidJpegPyramid.build(input(file, work), ledger) { progress ->
                if (progress.phase == TileBuildPhase.Complete) { entered.countDown(); check(release.await(10, TimeUnit.SECONDS)) }
            }
        }
        try {
            assertTrue(withContext(Dispatchers.IO) { entered.await(10, TimeUnit.SECONDS) })
            operation.cancel(); yield()
            assertFalse(operation.isCompleted)
            assertEquals(AndroidJpegPyramid.SESSION_ENVELOPE_BYTES, ledger.used)
        } finally {
            release.countDown()
            withContext(NonCancellable) { try { operation.await() } catch (_: CancellationException) { } }
        }
        try { assertEquals(0L, ledger.used); assertTrue(work.listFiles()!!.isEmpty()) }
        finally { dispose(root, work, file) }
    }

    @Test public fun returnedBuildKeepsChargeAndSlotUntilIdempotentClose(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        try {
            val result = AndroidJpegPyramid.build(input(file, work), ledger) { }
            try {
                assertEquals(1, ledger.calls)
                assertEquals(AndroidJpegPyramid.SESSION_ENVELOPE_BYTES, ledger.used)
                assertEquals("busy", refuse { AndroidJpegPyramid.build(input(file, work), ledger) { } }.reason)
            } finally { result.close(); result.close() }
            assertEquals(1, ledger.releases); assertEquals(0L, ledger.used)
        } finally { dispose(root, work, file) }
    }

    @Test public fun rootAdoptionGateKeepsFilesAndChargeUntilItsActualReaderReturns(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        val result = AndroidJpegPyramid.build(input(file, work), ledger) { }
        val directory = result.directory
        val entered = CountDownLatch(1); val release = CountDownLatch(1)
        val adoption = async(Dispatchers.Default) { result.withRootOutput { entered.countDown(); check(release.await(10, TimeUnit.SECONDS)) } }
        try {
            assertTrue(withContext(Dispatchers.IO) { entered.await(10, TimeUnit.SECONDS) })
            val closing = async(Dispatchers.Default) { result.close() }
            try {
                yield(); assertFalse(closing.isCompleted)
                assertTrue(directory.isDirectory)
                assertEquals(AndroidJpegPyramid.SESSION_ENVELOPE_BYTES, ledger.used)
            } finally { release.countDown(); adoption.await(); closing.await() }
            assertEquals(0L, ledger.used)
            assertEquals("build retired", refuse { result.withRootOutput { } }.reason)
        } finally { release.countDown(); result.close(); dispose(root, work, file) }
    }

    @Test public fun failedCleanupQuarantinesChargeAndSlotWithoutDeletingForeignFile(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        val result = AndroidJpegPyramid.build(input(file, work), ledger) { }
        val directory = result.directory
        val foreign = File(directory, "unrelated.txt").also { it.writeText("keep") }
        try {
            assertEquals("cleanup foreign entry", refuse { result.close() }.reason)
            assertEquals("keep", foreign.readText()); assertTrue(File(directory, ".owner").exists())
            assertEquals(AndroidJpegPyramid.SESSION_ENVELOPE_BYTES, ledger.used)
            assertEquals("busy", refuse { AndroidJpegPyramid.build(input(file, work), ledger) { } }.reason)
        } finally {
            check(foreign.delete()); result.close(); dispose(root, work, file)
        }
        assertEquals(0L, ledger.used)
    }

    @Test public fun failedNativeChargeReleaseRetainsQuarantineForExplicitRetry(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        val result = AndroidJpegPyramid.build(input(file, work), ledger) { }
        try {
            ledger.failRelease = true
            try { result.close(); fail("expected release failure") } catch (_: IllegalStateException) { }
            assertEquals(AndroidJpegPyramid.SESSION_ENVELOPE_BYTES, ledger.used)
            assertEquals("busy", refuse { AndroidJpegPyramid.build(input(file, work), ledger) { } }.reason)
        } finally {
            ledger.failRelease = false
            AndroidJpegPyramid.retryQuarantinedCleanup(); result.close()
            dispose(root, work, file)
        }
        assertEquals(0L, ledger.used); assertEquals(1, ledger.releases)
    }

    @Test public fun callbackFailureCleansOnlyOwnedOutputAndPreservesOriginal(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        val unrelated = File(work, "keep.txt").also { it.writeText("keep") }; val before = file.readBytes()
        try {
            try { AndroidJpegPyramid.build(input(file, work), ledger) { throw IllegalStateException("fixture callback") }; fail("expected callback failure") }
            catch (_: IllegalStateException) { }
            assertEquals(listOf(unrelated), work.listFiles()!!.toList())
            assertArrayEquals(before, file.readBytes()); assertEquals(0L, ledger.used)
        } finally { check(unrelated.delete()); dispose(root, work, file) }
    }

    @Test public fun sourceHashOrientationAndDiskBindingRefuseWithoutPublishing(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        try {
            assertEquals("source hash", refuse { AndroidJpegPyramid.build(input(file, work, digest = "0".repeat(64)), ledger) { } }.reason)
            assertEquals("source orientation", refuse { AndroidJpegPyramid.build(input(file, work, orientation = 2), ledger) { } }.reason)
            assertEquals("disk budget", refuse { AndroidJpegPyramid.build(input(file, work, disk = 1024), ledger) { } }.reason)
            assertTrue(work.listFiles()!!.isEmpty()); assertEquals(0L, ledger.used)
        } finally { dispose(root, work, file) }
    }

    @Test public fun explicitConsentAndBandAdmissionPrecedeAnySessionOrPlatformCall(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        try {
            assertEquals("untagged sRGB consent required", refuse { AndroidJpegPyramid.build(input(file, work, consent = false), ledger) { } }.reason)
            assertEquals("band budget", refuse { AndroidJpegPyramid.build(input(file, work, width = 32768, height = 256), ledger) { } }.reason)
            assertEquals(0, ledger.calls); assertTrue(work.listFiles()!!.isEmpty())
        } finally { dispose(root, work, file) }
    }

    @Test public fun taggedHdrAndNonJpegInputsNeverEnterProductionDecode(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        val before = file.readBytes()
        try {
            for ((marker, payload) in listOf(0xe2 to "ICC_PROFILE\u0000", 0xe2 to "MPF\u0000", 0xeb to "JUMBF HDR", 0xe1 to "http://ns.adobe.com/xap/1.0/")) {
                val body = payload.toByteArray(Charsets.US_ASCII)
                file.writeBytes(before.copyOfRange(0, 2) + byteArrayOf(0xff.toByte(), marker.toByte(), 0, (body.size + 2).toByte()) + body + before.copyOfRange(2, before.size))
                refuse { AndroidJpegPyramid.build(input(file, work), ledger) { fail("unexpected progress") } }
                assertTrue(work.listFiles()!!.isEmpty()); assertEquals(0L, ledger.used)
            }
            file.writeBytes(ByteArray(40) { 0 })
            assertEquals("JPEG required", refuse { AndroidJpegPyramid.build(input(file, work), ledger) { } }.reason)
        } finally { dispose(root, work, file) }
    }

    @Test public fun immutablePathReplacementIsRefusedEvenWhenBytesAreIdentical(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        val original = file.readBytes(); val backup = File(root, "retained-source.jpg")
        try {
            assertEquals("source changed", refuse {
                AndroidJpegPyramid.build(input(file, work), ledger) { progress ->
                    if (progress.phase == TileBuildPhase.Preview) { check(file.renameTo(backup)); file.writeBytes(original) }
                }
            }.reason)
            assertEquals(0L, ledger.used); assertTrue(work.listFiles()!!.isEmpty())
        } finally { if (file.exists()) check(file.delete()); check(backup.renameTo(file)); dispose(root, work, file) }
    }

    @Test public fun redirectedSourceAndWorkCannotBecomeProducerProvenance(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        val alias = File(root, "source-alias.jpg"); val workAlias = File(root, "work-alias")
        Files.createSymbolicLink(alias.toPath(), file.toPath()); Files.createSymbolicLink(workAlias.toPath(), work.toPath())
        try {
            assertEquals("source binding", refuse { AndroidJpegPyramid.build(input(alias, work), ledger) { } }.reason)
            assertEquals("private work", refuse { AndroidJpegPyramid.build(input(file, workAlias), ledger) { } }.reason)
            assertEquals(0L, ledger.used); assertTrue(work.listFiles()!!.isEmpty())
        } finally { check(alias.delete()); check(workAlias.delete()); dispose(root, work, file) }
    }

    @Test public fun oddPreviewUsesCeilingSampleAndRemainsWithinAdmittedEnvelope(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= 30)
        val root = root(); val work = work(root); val file = jpeg(root, 4097, 3); val ledger = Ledger()
        try {
            assertEquals(4, AndroidJpegPyramid.previewSample(4097, 3))
            val result = AndroidJpegPyramid.build(input(file, work, 4097, 3), ledger) { }
            try {
                assertTrue(maxOf(result.previewWidth, result.previewHeight) <= 2048)
                val preview = checkNotNull(BitmapFactory.decodeFile(File(result.directory, "preview.jpg").path))
                try {
                    assertEquals(result.previewWidth, preview.width); assertEquals(result.previewHeight, preview.height)
                    assertTrue(preview.allocationByteCount <= AndroidJpegPyramid.MAX_PREVIEW_BYTES)
                } finally { preview.recycle() }
            } finally { result.close() }
            assertEquals(0L, ledger.used)
        } finally { dispose(root, work, file) }
    }

    @Test public fun postAllocationAdmissionRejectsAlphaDepthProfileAndExcessPreview(): Unit {
        val transparent = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888)
        val wide = Bitmap.createBitmap(2049, 1, Bitmap.Config.ARGB_8888)
        val deep = Bitmap.createBitmap(8, 8, Bitmap.Config.RGBA_F16)
        val tagged = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888, false, ColorSpace.get(ColorSpace.Named.DISPLAY_P3))
        try {
            try { AndroidJpegPyramid.admitBitmap(transparent, 256); fail("expected alpha refusal") }
            catch (error: TileBuildFailure) { assertEquals("nonopaque bitmap", error.reason) }
            try { AndroidJpegPyramid.admitPreview(wide); fail("expected extent refusal") }
            catch (error: TileBuildFailure) { assertEquals("preview extent", error.reason) }
            try { AndroidJpegPyramid.admitBitmap(deep, 1024); fail("expected depth refusal") }
            catch (error: TileBuildFailure) { assertEquals("decoded bitmap format", error.reason) }
            try { AndroidJpegPyramid.admitBitmap(tagged, 1024); fail("expected profile refusal") }
            catch (error: TileBuildFailure) { assertEquals("decoded bitmap format", error.reason) }
        } finally { transparent.recycle(); wide.recycle(); deep.recycle(); tagged.recycle() }
    }

    @Test public fun stridePaddingAndOverflowCannotBypassActualAllocationAdmission(): Unit {
        AndroidJpegPyramid.admitLayout(3, 2, 16, 32, 32)
        for (layout in listOf(longArrayOf(3, 2, 16, 24, 32), longArrayOf(3, 2, 16, 64, 32),
            longArrayOf(3, 2, Long.MAX_VALUE, Long.MAX_VALUE, Long.MAX_VALUE), longArrayOf(0, 2, 16, 32, 32))) {
            try { AndroidJpegPyramid.admitLayout(layout[0].toInt(), layout[1].toInt(), layout[2], layout[3], layout[4]); fail("expected layout refusal") }
            catch (error: TileBuildFailure) { assertEquals("decoded bitmap budget", error.reason) }
        }
    }

    @Test public fun sdkBelowThirtyRefusesBeforeSessionAdmission(): Unit = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT < 30)
        val root = root(); val work = work(root); val file = jpeg(root); val ledger = Ledger()
        try {
            assertEquals("SDK30 required", refuse { AndroidJpegPyramid.build(input(file, work), ledger) { } }.reason)
            assertEquals(0, ledger.calls)
        } finally { dispose(root, work, file) }
    }
}
