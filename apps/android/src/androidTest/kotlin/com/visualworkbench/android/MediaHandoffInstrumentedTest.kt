package com.visualworkbench.android

import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.net.Uri
import android.provider.MediaStore
import android.system.Os
import androidx.core.content.FileProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.*
import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import kotlinx.coroutines.runBlocking

@RunWith(AndroidJUnit4::class)
class MediaHandoffInstrumentedTest {
    private class Fixture : AutoCloseable {
        val actual = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(actual.cacheDir.canonicalFile, "handoff-fixture-${UUID.randomUUID()}").also { check(it.mkdir()) }
        val context = object : ContextWrapper(actual) {
            override fun getApplicationContext(): Context = this
            override fun getCacheDir(): File = root
            override fun getFilesDir(): File = File(root, "files").also { check(it.isDirectory || it.mkdir()) }
            override fun revokeUriPermission(uri: Uri, modeFlags: Int) { /* synthetic authority has no recipient */ }
        }
        val handoff = MediaHandoff(context) { _, file -> Uri.parse("content://synthetic.handoff/${file.parentFile?.name}/${file.name}") }
        override fun close() { assertEquals(actual.cacheDir.canonicalFile, root.canonicalFile.parentFile); assertTrue(root.deleteRecursively()) }
    }

    @Suppress("DEPRECATION")
    @Test fun cameraUsesFullResolutionOutputAndNarrowReadWriteGrants() {
        val uri = Uri.parse("content://synthetic.handoff/capture")
        val intent = MediaIntents.camera(uri)
        assertEquals(MediaStore.ACTION_IMAGE_CAPTURE, intent.action)
        assertEquals(uri, intent.getParcelableExtra<Uri>(MediaStore.EXTRA_OUTPUT))
        assertEquals(uri, intent.clipData?.getItemAt(0)?.uri)
        assertEquals(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION, intent.flags)
        assertFalse(intent.hasExtra("data"))
    }

    @Suppress("DEPRECATION")
    @Test fun pngShareContainsOnlyImageAndTemporaryReadGrant() {
        val uri = Uri.parse("content://synthetic.handoff/image")
        val intent = MediaIntents.sharePng(uri)
        assertEquals(Intent.ACTION_SEND, intent.action); assertEquals("image/png", intent.type)
        assertEquals(uri, intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))
        assertEquals(uri, intent.clipData?.getItemAt(0)?.uri)
        assertEquals(Intent.FLAG_GRANT_READ_URI_PERMISSION, intent.flags)
        assertFalse(intent.hasExtra(Intent.EXTRA_TEXT)); assertFalse(intent.hasExtra(Intent.EXTRA_HTML_TEXT))
        assertEquals(uri, MediaIntents.incoming(intent))
    }

    @Test fun intakeRejectsFilesystemAndNonImageOrMultipleShares() {
        assertNull(MediaIntents.incoming(Intent(Intent.ACTION_SEND).setType("text/plain")))
        assertNull(MediaIntents.incoming(Intent(Intent.ACTION_SEND_MULTIPLE).setType("image/png")))
        val invalid = Intent(Intent.ACTION_SEND).setType("image/png").putExtra(Intent.EXTRA_STREAM, Uri.parse("file:///private/original.png"))
        assertTrue(runCatching { MediaIntents.incoming(invalid) }.exceptionOrNull() is IllegalArgumentException)
    }

    @Test fun realFileProviderExposesHandoffButRejectsProjectsAndScratch() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val provider = context.packageManager.resolveContentProvider("${context.packageName}.images", 0)
        assertNotNull(provider); assertFalse(checkNotNull(provider).exported); assertTrue(provider.grantUriPermissions)
        val handoff = MediaHandoff(context)
        val lease = handoff.create(camera = true)
        try {
            context.contentResolver.openOutputStream(lease.uri, "wt")!!.use { it.write(byteArrayOf(9, 8, 7)) }
            assertArrayEquals(byteArrayOf(9, 8, 7), lease.file.readBytes())
            for (file in listOf(File(context.filesDir, "projects/synthetic/project.sqlite"), File(context.cacheDir, "raster-transfers/synthetic/original.bin"))) {
                assertTrue(runCatching { FileProvider.getUriForFile(context, "${context.packageName}.images", file) }.isFailure)
            }
        } finally { handoff.remove(lease, camera = true) }
    }

    @Test fun expiredCleanupPreservesFreshSharesActiveCameraAndUnrelatedFiles() {
        Fixture().use { f ->
            val old = f.handoff.create(false); val fresh = f.handoff.create(false); val camera = f.handoff.create(true)
            val sentinel = File(f.root, "unrelated.txt").also { it.writeText("preserve") }
            val now = System.currentTimeMillis()
            assertTrue(old.file.setLastModified(now - MediaHandoff.RETENTION_MS - 1_000))
            assertTrue(camera.file.setLastModified(now - MediaHandoff.RETENTION_MS - 1_000))
            f.handoff.prune(now, camera.token)
            assertFalse(old.file.exists()); assertTrue(fresh.file.exists()); assertTrue(camera.file.exists())
            assertEquals("preserve", sentinel.readText())
            f.handoff.remove(fresh, false); f.handoff.remove(camera, true)
        }
    }

    @Test fun cleanupRefusesSymlinkToUnrelatedOriginal() {
        Fixture().use { f ->
            val original = File(f.root, "original.png").also { it.writeText("preserve") }
            val lease = f.handoff.create(false)
            assertTrue(lease.file.delete()); Os.symlink(original.absolutePath, lease.file.absolutePath)
            assertTrue(runCatching { f.handoff.remove(lease, false) }.isFailure)
            assertEquals("preserve", original.readText())
            assertTrue(lease.file.delete()); assertTrue(checkNotNull(lease.file.parentFile).delete())
        }
    }

    @Test fun exportReceiptBecomesStaleOnEditEvenWithoutHostSequenceChange() {
        val info = ProjectInfo("project", "synthetic", "device", 1uL, false, false, 0uL, "old", listOf("document"))
        val doc = DocumentSnapshot("document", "synthetic", 80u, 60u, 8u, emptyList(), RenderList(info, emptyList()))
        val ready = ExportReady("project", "document", "old", 0uL, 80u, 60u, "hash")
        assertTrue(ready.matches(info, doc))
        assertFalse(ready.matches(info.copy(stateHash = "optimistic-edit"), doc))
        assertFalse(ready.matches(info.copy(hostSeq = 1uL), doc))
        assertFalse(ready.matches(info.copy(projectId = "other"), doc))
        assertFalse(ready.matches(info, doc.copy(documentId = "other")))
    }

    @Test fun jpegMatteDoesNotCarryIntoTransparentFormats() {
        val jpeg = ExportSelection(encoding = ExportEncoding.Jpeg, whiteMatte = true)
        assertEquals(0xffffffu, jpeg.matteRgb)
        for (format in listOf(ExportEncoding.Png8, ExportEncoding.Png16, ExportEncoding.WebpLossless, ExportEncoding.WebpLossy))
            assertNull(jpeg.copy(encoding = format).matteRgb)
    }

    @Test fun destinationTicketRejectsRecheckedFormatProjectOrRevisionChanges() {
        val info = ProjectInfo("project", "synthetic", "device", 1uL, false, false, 2uL, "state", listOf("document"))
        val doc = DocumentSnapshot("document", "synthetic", 80u, 60u, 8u, emptyList(), RenderList(info, emptyList()))
        val options = ExportOptions("document", ImageFormat.Png8)
        val ticket = ExportTicket("ticket", "project", "document", 2uL, "state", options, ExportEncoding.Png8)
        assertTrue(ticket.matches(info, doc, options))
        assertFalse(ticket.matches(info, doc, options.copy(format = ImageFormat.Jpeg(90u))))
        assertFalse(ticket.matches(info.copy(projectId = "other"), doc, options))
        assertFalse(ticket.matches(info.copy(hostSeq = 3uL), doc, options))
        assertFalse(ticket.matches(info.copy(stateHash = "new"), doc, options))
    }

    @Test fun completedCameraOriginalSurvivesExpiryAndCleanupFailureIsReported() = runBlocking {
        Fixture().use { f ->
            val lease = f.handoff.create(true)
            lease.file.writeText("only original")
            assertTrue(lease.file.setLastModified(1_000))
            f.handoff.prune(System.currentTimeMillis())
            assertEquals("only original", lease.file.readText())
            assertEquals(lease.token, f.handoff.captures().single().token)
            var reported = false
            assertFalse(cleanupHandoff({ throw SecurityException("injected revoke failure") }) { reported = true })
            assertTrue(reported); assertEquals("only original", lease.file.readText())
            f.handoff.remove(lease, true)
        }
    }

    @Test fun failedLeaseCreationRollsBackOnlyItsEmptyOwnedDirectory() {
        Fixture().use { f ->
            val preserved = f.handoff.create(true).also { it.file.writeText("original") }
            val failing = MediaHandoff(f.context) { _, _ -> throw IllegalStateException("injected URI failure") }
            assertTrue(runCatching { failing.create(true) }.isFailure)
            assertEquals(listOf(preserved.token), File(f.context.filesDir, "camera-inbox").listFiles().orEmpty().map { it.name })
            assertEquals("original", preserved.file.readText())
            f.handoff.remove(preserved, true)
        }
    }

    @Test fun rotatedViewCropUsesClippedIntegerDocumentBoundsAndRejectsEmptyViews() {
        val rect = viewExportRegion(listOf(Point(-3.4, 4.9), Point(40.2, -8.0), Point(90.1, 45.2), Point(31.0, 75.4)), 80u, 60u)
        assertEquals(Rect(0.0, 0.0, 80.0, 60.0), rect)
        assertTrue(runCatching { viewExportRegion(List(4) { Point(-4.0, -4.0) }, 80u, 60u) }.isFailure)
        assertTrue(runCatching { viewExportRegion(List(4) { Point(Double.NaN, 0.0) }, 80u, 60u) }.isFailure)
    }
}
