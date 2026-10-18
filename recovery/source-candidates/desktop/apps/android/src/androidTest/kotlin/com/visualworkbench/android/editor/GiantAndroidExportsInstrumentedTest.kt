package com.visualworkbench.android.editor

import android.content.*
import android.content.pm.ProviderInfo
import android.database.Cursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.UUID

class GiantAndroidExportsInstrumentedTest {
    @Test fun checkRendersOneImmutableSnapshotAndSaveShareUseThoseExactBytes() = runBlocking {
        Fixture().use { f ->
            val adapter = f.adapter()
            try {
                val ready = adapter.prepare(binding(), 10u, 5u, options()) { }
                assertEquals(10u, ready.width); assertFalse(ready.requiresRenderValidation); assertEquals(1, f.renders)
                val saved = adapter.save(f.uri, binding(), options()) { }
                assertArrayEquals(f.bytes, f.destination.readBytes()); assertEquals(1, f.opens)
                val shared = adapter.share(binding(), options()) { }
                try { assertArrayEquals(f.bytes, shared.first.file.readBytes()); assertEquals(saved, shared.second) }
                finally { f.handoff.remove(shared.first, camera = false) }
                assertEquals(1, f.renders)
            } finally { adapter.clear() }
        }
    }
    @Test fun staleProjectDocumentHashSequenceOrOptionsNeverOpenProviderOrCreateShareLease() = runBlocking {
        Fixture().use { f ->
            val adapter = f.adapter()
            try {
                adapter.prepare(binding(), 10u, 5u, options()) { }
                for (bad in listOf(binding().copy(projectId = "other"), binding().copy(documentId = "other"),
                    binding().copy(hostSeq = 8uL), binding().copy(stateHash = "c".repeat(64)))) {
                    try { adapter.save(f.uri, bad, options()) { }; fail("stale admitted") } catch (_: IllegalArgumentException) { }
                    try { adapter.share(bad, options()) { }; fail("stale share admitted") } catch (_: IllegalArgumentException) { }
                }
                try { adapter.share(binding(), options().copy(marked = false)) { }; fail("different settings admitted") } catch (_: IllegalArgumentException) { }
                assertEquals(0, f.opens); assertEquals(0, f.shareFactories)
            } finally { adapter.clear() }
        }
    }
    @Test fun foreignNativeReceiptIsRefusedAndPrivateScratchIsRemoved() = runBlocking {
        Fixture().use { f ->
            val adapter = f.adapter(response = { f.receipt().copy(revision = info().copy(stateHash = "f".repeat(64))) })
            try { adapter.prepare(binding(), 10u, 5u, options()) { }; fail("foreign native receipt") }
            catch (_: IllegalArgumentException) { }
            finally { adapter.clear() }
            assertTrue(File(f.root, "raster-transfers").listFiles().isNullOrEmpty()); assertEquals(0, f.opens)
        }
    }
    @Test fun sameLengthPrivateFileChangeIsCaughtBeforeProviderPublication() = runBlocking {
        Fixture().use { f ->
            val adapter = f.adapter()
            try {
                adapter.prepare(binding(), 10u, 5u, options()) { }
                checkNotNull(f.output).writeBytes(ByteArray(f.bytes.size) { 9 })
                try { adapter.save(f.uri, binding(), options()) { }; fail("changed private bytes") } catch (_: IllegalArgumentException) { }
                assertEquals(0, f.opens)
            } finally { adapter.clear() }
        }
    }
    @Test fun cancelledNativePreparationAndClearJoinBeforePrivateRemoval() = runBlocking {
        Fixture().use { f ->
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            val adapter = f.adapter(response = { withContext(NonCancellable) { entered.complete(Unit); release.await() }; f.receipt() })
            val preparation = launch { adapter.prepare(binding(), 10u, 5u, options()) { } }
            var clearing: Job? = null
            try {
                withTimeout(3000) { entered.await() }; preparation.cancel()
                clearing = launch(start = CoroutineStart.UNDISPATCHED) { adapter.clear() }
                assertFalse(preparation.isCompleted); assertFalse(checkNotNull(clearing).isCompleted); assertTrue(checkNotNull(f.output).exists())
            } finally { release.complete(Unit); withContext(NonCancellable) { preparation.join(); clearing?.join(); adapter.clear() } }
            assertFalse(checkNotNull(f.output).exists()); assertEquals(0, f.opens)
        }
    }
    @Test fun cancelledSaveWaitingForPreparationOwnsAndDeletesItsNewDestination() = runBlocking {
        Fixture().use { f ->
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            val adapter = f.adapter(response = { entered.complete(Unit); release.await(); f.receipt() })
            val preparation = launch { adapter.prepare(binding(), 10u, 5u, options()) { } }
            var saving: Job? = null
            try {
                withTimeout(3000) { entered.await() }
                saving = launch(start = CoroutineStart.UNDISPATCHED) { adapter.save(f.uri, binding(), options()) { } }
                checkNotNull(saving).cancel(); withTimeout(3000) { checkNotNull(saving).join() }
                assertEquals(1, f.deletes); assertEquals(0, f.opens)
            } finally { release.complete(Unit); withContext(NonCancellable) { saving?.join(); preparation.join(); adapter.clear() } }
        }
    }
    @Test fun insufficientPrivateSpaceRefusesBeforeNativeExport() = runBlocking {
        Fixture().use { f ->
            val adapter = f.adapter(space = { 1 })
            try { adapter.prepare(binding(), 10u, 5u, options()) { }; fail("space admitted") }
            catch (_: IllegalArgumentException) { } finally { adapter.clear() }
            assertEquals(0, f.renders); assertEquals(0, f.opens)
        }
    }
    @Test fun pngRegionAndShareLimitsRefuseImplicitConversionsAndFractionalCrops() {
        assertEquals(3u to 2u, giantPngDimensions(binding(), 10u, 5u, options().copy(region = Rect(1.0, 2.0, 3.0, 2.0))))
        for (bad in listOf(options().copy(format = ImageFormat.Png16), options().copy(convertToSrgb = true), options().copy(allowDepthReduction = true),
            options().copy(matteRgb = 0xffffffu), options().copy(assumeUntaggedSrgb = false),
            options().copy(region = Rect(0.5, 0.0, 3.0, 2.0)), options().copy(region = Rect(9.0, 0.0, 2.0, 1.0)))) {
            try { giantPngDimensions(binding(), 10u, 5u, bad); fail("unsupported option") } catch (_: IllegalArgumentException) { }
        }
        admitGiantShare(GIANT_PNG_SHARE_LIMIT.toULong())
        try { admitGiantShare(GIANT_PNG_SHARE_LIMIT.toULong() + 1uL); fail("share cap") } catch (_: IllegalArgumentException) { }
    }
    @Test fun rendererPauseKeepsSurfaceAdmissionSealedUntilEncodingSettlement() = runBlocking {
        var available = 0
        val leases = GiantRenderLeases { available++ }
        val old = checkNotNull(leases.acquire { })
        val pause = leases.pauseAdmission()
        val retirement = async(start = CoroutineStart.UNDISPATCHED) { leases.retireAll() }
        try {
            old.released(); retirement.await()
            assertEquals(0, available); assertNull(leases.acquire { })
            pause.close(); assertEquals(1, available)
            val next = checkNotNull(leases.acquire { }); next.released(announce = false)
            pause.close(); assertEquals(1, available)
        } finally { old.released(); withContext(NonCancellable) { retirement.await() }; pause.close() }
    }

    private class Fixture : AutoCloseable {
        val actual = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(actual.cacheDir, "giant-export-${UUID.randomUUID()}").apply { check(mkdir()) }.canonicalFile
        val destination = File(root, "destination")
        val uri: Uri = Uri.parse("content://vw.giant.export/image")
        val bytes = byteArrayOf(1, 2, 3, 4) // Native boundary is injected; no fake codec acceptance.
        var output: File? = null; var renders = 0; var opens = 0; var deletes = 0; var shareFactories = 0
        private lateinit var resolver: ContentResolver
        val context = object : ContextWrapper(actual) {
            override fun getApplicationContext(): Context = this
            override fun getCacheDir() = root
            override fun getFilesDir() = root
            override fun getContentResolver() = resolver
            override fun revokeUriPermission(uri: Uri?, modeFlags: Int) { }
        }
        val handoff = MediaHandoff(context) { _, file -> Uri.parse("content://vw.giant.export/share/${file.parentFile?.name}") }
        init {
            val provider = object : ContentProvider() {
                override fun onCreate() = true
                override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? = null
                override fun getType(uri: Uri) = "image/png"
                override fun insert(uri: Uri, values: ContentValues?): Uri? = error("unused")
                override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = error("unused")
                override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int { check(uri == this@Fixture.uri); deletes++; destination.delete(); return 1 }
                override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
                    check(uri == this@Fixture.uri && mode == "wt"); opens++
                    return ParcelFileDescriptor.open(destination, ParcelFileDescriptor.MODE_CREATE or ParcelFileDescriptor.MODE_TRUNCATE or ParcelFileDescriptor.MODE_WRITE_ONLY)
                }
            }
            provider.attachInfo(context, ProviderInfo().apply { authority = "vw.giant.export"; exported = false })
            resolver = ContentResolver.wrap(provider)
        }
        fun receipt() = GiantPngReceipt(info(), bytes.size.toULong(), "b".repeat(64), "{}", 1024uL)
        fun adapter(response: suspend () -> GiantPngReceipt = { receipt() }, space: (File) -> Long = { Long.MAX_VALUE }) = GiantAndroidExports(context,
            render = { request -> renders++; output = File(request.outputPath).also { it.writeBytes(bytes) }; response() },
            handoffFactory = { shareFactories++; handoff }, availableBytes = space)
        override fun close() {
            assertEquals(actual.cacheDir.canonicalFile, root.parentFile)
            assertTrue(root.name.startsWith("giant-export-")); assertTrue(root.deleteRecursively())
        }
    }
    private companion object {
        fun info() = ProjectInfo("project", "Synthetic", "device", 1uL, false, false, 7uL, "a".repeat(64), listOf("document"))
        fun binding() = WorkflowBinding("project", "document", 7uL, "a".repeat(64))
        fun options() = ExportOptions("document", assumeUntaggedSrgb = true)
    }
}
