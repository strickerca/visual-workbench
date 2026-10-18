package com.visualworkbench.android.editor

import android.content.*
import android.content.pm.ProviderInfo
import android.database.Cursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.UUID

class GiantAndroidInputsInstrumentedTest {
    @Test fun providerCopyIsExactAndNeverUsesAnExternalCodecPath() = runBlocking {
        Fixture().use { f ->
            val output = File(f.root, "copy")
            withContext(Dispatchers.IO) { copyGiantOriginal(ProviderIo(f.context.contentResolver), f.uri, output, 16) }
            assertArrayEquals(f.original.readBytes(), output.readBytes()); assertEquals(1, f.opens)
        }
    }
    @Test fun encodedLimitRefusesBeforeWritingTheOversizedChunk() = runBlocking {
        Fixture().use { f ->
            val output = File(f.root, "copy")
            try {
                withContext(Dispatchers.IO) { copyGiantOriginal(ProviderIo(f.context.contentResolver), f.uri, output, 2) }
                fail("limit was ignored")
            } catch (_: IllegalArgumentException) { }
            assertTrue(output.length() <= 2); assertArrayEquals(byteArrayOf(1, 2, 3, 4), f.original.readBytes())
        }
    }
    @Test fun nonContentUriNeverOpensProviderOrAdmitsNativeSource() = runBlocking {
        Fixture().use { f ->
            var prepare = 0
            val inputs = GiantAndroidInputs(f.context, prepare = { prepare++; error("must not run") })
            try { inputs.importJpeg(Uri.parse("file:///unowned.jpg"), { _, _ -> error("unused") }) { }; fail("scheme admitted") }
            catch (_: IllegalArgumentException) { }
            assertEquals(0, prepare); assertEquals(0, f.opens)
        }
    }
    @Test fun publishReturnsOnlyAfterProducerAndSourceCleanup() = runBlocking {
        Fixture().use { f ->
            val events = mutableListOf<String>(); val project = Project(events)
            val source = Source(f, project, events)
            val inputs = inputs(f, source, events)
            val delivered = inputs.importJpeg(f.uri, { _, _ -> f.options() }) { }
            assertSame(project, delivered)
            assertEquals(listOf("build", "publish", "tiles:close", "source:close"), events)
            assertTrue(File(f.root, "raster-transfers").listFiles().isNullOrEmpty())
            delivered.close(); assertEquals("project:close", events.last())
        }
    }
    @Test fun cancellationDuringFinalCleanupClosesUndeliveredProjectAndJoins() = runBlocking {
        Fixture().use { f ->
            val events = mutableListOf<String>(); val project = Project(events)
            val entered = CompletableDeferred<Unit>(); val leave = CompletableDeferred<Unit>()
            val source = Source(f, project, events).apply { closing = { entered.complete(Unit); leave.await() } }
            val inputs = inputs(f, source, events)
            val job = launch { inputs.importJpeg(f.uri, { _, _ -> f.options() }) { } }
            try {
                withTimeout(3000) { entered.await() }
                job.cancel(); yield()
                assertFalse(job.isCompleted); assertFalse(project.closed)
            } finally { leave.complete(Unit); withContext(NonCancellable) { withTimeout(3000) { job.join(); inputs.settle() } } }
            assertTrue(project.closed)
            assertTrue(events.indexOf("tiles:close") < events.indexOf("source:close"))
            assertTrue(File(f.root, "raster-transfers").listFiles().isNullOrEmpty())
        }
    }
    @Test fun insufficientWorkingStorageNeverBuildsOrPublishes() = runBlocking {
        Fixture().use { f ->
            val events = mutableListOf<String>(); val source = Source(f, Project(events), events)
            val inputs = GiantAndroidInputs(f.context, prepare = { source },
                build = { _, _, _ -> error("build after refusal") }, availableBytes = { 1 })
            try { inputs.importJpeg(f.uri, { _, _ -> f.options() }) { }; fail("storage admitted") }
            catch (_: IllegalArgumentException) { }
            assertEquals(listOf("source:close"), events)
            assertTrue(File(f.root, "raster-transfers").listFiles().isNullOrEmpty())
        }
    }
    @Test fun workingAdmissionRejectsUnsupportedSmallWideAndMismatchedInputs() {
        val good = info("private")
        assertTrue(giantPhoneWorkingBytes(good) > 1_000_000_000)
        for (bad in listOf(good.copy(format = "png"), good.copy(width = 100u, height = 100u, rawWidth = 100u, rawHeight = 100u),
            good.copy(width = 20000u, height = 4000u, rawWidth = 20000u, rawHeight = 4000u),
            good.copy(orientation = 6u), good.copy(encodedBytes = GIANT_PHONE_ORIGINAL_BYTES.toULong() + 1uL))) {
            try { giantPhoneWorkingBytes(bad); fail("bad metadata admitted") } catch (_: IllegalArgumentException) { }
        }
    }
    @Test fun uncertainCleanupKeepsOwnerUntilExplicitSettlement() = runBlocking {
        Fixture().use { f ->
            val events = mutableListOf<String>(); val project = Project(events)
            var attempts = 0
            val source = Source(f, project, events).apply { closing = { if (++attempts <= 2) throw IllegalStateException("controlled close refusal") } }
            val inputs = inputs(f, source, events)
            try { inputs.importJpeg(f.uri, { _, _ -> f.options() }) { }; fail("cleanup accepted") }
            catch (_: IllegalStateException) { }
            assertTrue(project.closed)
            assertFalse(File(f.root, "raster-transfers").listFiles().isNullOrEmpty())
            inputs.settle()
            assertTrue(File(f.root, "raster-transfers").listFiles().isNullOrEmpty()); assertEquals(3, attempts)
        }
    }

    private fun inputs(f: Fixture, source: Source, events: MutableList<String>) = GiantAndroidInputs(f.context, prepare = { source },
        build = { _, dir, _ -> events += "build"; object : GiantBuiltCache {
            override val directory = dir.absolutePath
            override suspend fun close() { events += "tiles:close" }
        } }, availableBytes = { Long.MAX_VALUE })
    private class Source(val f: Fixture, val project: Project, val events: MutableList<String>) : GiantPreparedSource {
        var closing: suspend () -> Unit = { }
        override suspend fun describe() = info(f.original.absolutePath)
        override suspend fun publish(options: GiantProjectOptions, approvedProducerDirectory: String): WorkbenchProject {
            events += "publish"; return project
        }
        override suspend fun rebuild(project: WorkbenchProject, documentId: String, approvedProducerDirectory: String, diskCacheBytes: ULong) = error("unused")
        override suspend fun close() { events += "source:close"; closing() }
    }
    private class Project(val events: MutableList<String>) : WorkbenchProject {
        var closed = false
        override val changes = emptyFlow<ProjectChange>()
        override suspend fun info(): ProjectInfo = error("unused")
        override suspend fun document(documentId: String): DocumentSnapshot = error("unused")
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
        override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
        override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
        override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
        override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("unused")
        override suspend fun export(options: ExportOptions): ExportResult = error("unused")
        override suspend fun close() { if (!closed) { closed = true; events += "project:close" } }
    }
    private class Fixture : AutoCloseable {
        private val actual = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(actual.cacheDir.canonicalFile, "giant-input-${UUID.randomUUID()}").also { check(it.mkdir()) }
        val original = File(root, "original.jpg").also { it.writeBytes(byteArrayOf(1, 2, 3, 4)) }
        val uri: Uri = Uri.parse("content://vw.giant.fixture/original")
        var opens = 0
        private lateinit var resolver: ContentResolver
        val context = object : ContextWrapper(actual) {
            override fun getApplicationContext(): Context = this
            override fun getCacheDir(): File = root
            override fun getContentResolver(): ContentResolver = resolver
        }
        init {
            val provider = object : ContentProvider() {
                override fun onCreate() = true
                override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? = null
                override fun getType(uri: Uri) = "image/jpeg"
                override fun insert(uri: Uri, values: ContentValues?): Uri? = error("read only")
                override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = error("read only")
                override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = error("read only")
                override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
                    check(uri == this@Fixture.uri && mode == "r"); opens++
                    return ParcelFileDescriptor.open(original, ParcelFileDescriptor.MODE_READ_ONLY)
                }
            }
            provider.attachInfo(context, ProviderInfo().apply { authority = "vw.giant.fixture"; exported = false })
            resolver = ContentResolver.wrap(provider)
        }
        fun options() = GiantProjectOptions(File(root, "published-project").absolutePath, "project", "document", "layer", "device", "Synthetic", 1, GIANT_PHONE_WORK_BYTES.toULong())
        override fun close() {
            assertEquals(actual.cacheDir.canonicalFile, root.canonicalFile.parentFile)
            assertTrue(root.name.startsWith("giant-input-")); assertTrue(root.deleteRecursively())
        }
    }
    private companion object {
        fun info(path: String) = GiantInputInfo(path, "a".repeat(64), "b".repeat(64), 4uL, 8500u, 6000u, 8500u, 6000u, 1u, "jpeg")
    }
}
