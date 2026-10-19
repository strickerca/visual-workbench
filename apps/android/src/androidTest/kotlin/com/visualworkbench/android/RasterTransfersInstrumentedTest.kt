package com.visualworkbench.android

import android.content.ContentProvider
import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.ProviderInfo
import android.database.Cursor
import android.database.MatrixCursor
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.RasterTransfers
import com.visualworkbench.android.editor.MediaHandoff
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger

@RunWith(AndroidJUnit4::class)
class RasterTransfersInstrumentedTest {
    private class Fixture : AutoCloseable {
        val actual: Context = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(actual.cacheDir.canonicalFile,"stream-fixture-${UUID.randomUUID()}").also { check(it.mkdir()) }
        val source = File(root,"source.png")
        val destination = File(root,"destination.png")
        lateinit var resolver: ContentResolver
        val authority = "synthetic.raster.fixture"
        val input: Uri = Uri.parse("content://$authority/source")
        val output: Uri = Uri.parse("content://$authority/output")
        private var inputOverride: ParcelFileDescriptor? = null
        private var outputOverride: ParcelFileDescriptor? = null
        private val pipeEnds = mutableListOf<ParcelFileDescriptor>()
        val opened = CompletableDeferred<ParcelFileDescriptor>()
        val outputWriterOpens = AtomicInteger()
        var displayName = "Synthetic original.png"
        var mediaType = "image/png"
        fun stallInput() { val pipe = ParcelFileDescriptor.createPipe(); pipeEnds.addAll(pipe); inputOverride = pipe[0] }
        fun stallOutput() { val pipe = ParcelFileDescriptor.createPipe(); pipeEnds.addAll(pipe); outputOverride = pipe[1] }
        val context = object : ContextWrapper(actual) {
            override fun getApplicationContext(): Context = this
            override fun getCacheDir(): File = root
            override fun getContentResolver() = resolver
            override fun revokeUriPermission(uri: Uri, modeFlags: Int) { /* synthetic authority has no recipient */ }
        }
        init {
            val bitmap=Bitmap.createBitmap(63,49,Bitmap.Config.ARGB_8888)
            try { bitmap.eraseColor(0xff456789.toInt()); source.writeBytes(ByteArrayOutputStream().also { check(bitmap.compress(Bitmap.CompressFormat.PNG,100,it)) }.toByteArray()) }
            finally { bitmap.recycle() }
            val provider=object:ContentProvider(){
                override fun onCreate()=true
                override fun query(uri:Uri,projection:Array<out String>?,selection:String?,selectionArgs:Array<out String>?,sortOrder:String?):Cursor = MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME)).also { it.addRow(arrayOf(displayName)) }
                override fun getType(uri:Uri)=mediaType
                override fun insert(uri:Uri,values:ContentValues?):Uri?=null
                override fun update(uri:Uri,values:ContentValues?,selection:String?,selectionArgs:Array<out String>?)=0
                override fun delete(uri:Uri,selection:String?,selectionArgs:Array<out String>?):Int = if(uri==output&&destination.delete())1 else 0
                override fun openFile(uri:Uri,mode:String):ParcelFileDescriptor {
                    if (uri == output && 'w' in mode) outputWriterOpens.incrementAndGet()
                    val injected = if (uri == input) inputOverride else outputOverride
                    val descriptor = if (injected != null) ParcelFileDescriptor.dup(injected.fileDescriptor)
                        else ParcelFileDescriptor.open(if(uri==input)source else destination,ParcelFileDescriptor.parseMode(mode))
                    opened.complete(descriptor)
                    return descriptor
                }
            }
            provider.attachInfo(context,ProviderInfo().apply { authority=this@Fixture.authority;exported=false })
            // API 29 public provider wrapper avoids the legacy android.test library.
            resolver = ContentResolver.wrap(provider)
        }
        fun assertNoWorkspaces(){val transfers=File(root,"raster-transfers");assertTrue(!transfers.exists()||transfers.listFiles()?.isEmpty()==true)}
        override fun close(){pipeEnds.forEach { it.close() };assertEquals(actual.cacheDir.canonicalFile,root.canonicalFile.parentFile);assertTrue(root.deleteRecursively())}
    }
    @Test fun fileImportAndExportUseRealCoreAndCleanPrivateScratch() = runBlocking {
        Fixture().use { fixture ->
            val core=workbenchCore();val streaming=core as WorkbenchStreamingCore;val now=System.currentTimeMillis();val id=core.newId(now.toULong());
            val original=fixture.source.readBytes();val stages=mutableListOf<String>();val transfers=RasterTransfers(fixture.context)
            val project=transfers.importImage(fixture.input,streaming,{source,work,title->CreateFileProject(File(fixture.root,"project").absolutePath,id,core.newId(now.toULong()),core.newId(now.toULong()),core.newDeviceId(),title,source,work,now)}) { stages += it }
            try {
                fixture.assertNoWorkspaces();val info=project.info();fixture.destination.createNewFile()
                val result=transfers.export(fixture.output,project as WorkbenchStreamingProject,ExportOptions(info.documentIds.single(),marked=false),
                    { receipt -> assertEquals(info.projectId,receipt.revision.projectId) }) { stages += it }
                assertEquals(result.encodedBytes,fixture.destination.length().toULong());assertArrayEquals(original,fixture.source.readBytes())
                val pixels=BitmapFactory.decodeFile(fixture.destination.absolutePath)
                try { assertEquals(63,pixels.width);assertEquals(49,pixels.height);assertEquals(0xff456789.toInt(),pixels.getPixel(30,20)) } finally { pixels.recycle() }
                assertTrue(stages.any{it.contains("Rendering")});assertTrue(stages.any{it.contains("Saving")});fixture.assertNoWorkspaces()
            } finally { withContext(NonCancellable){project.close()} }
        }
    }
    @Test fun unsupportedRawAndAiNeverReachAnImageDecoderOrOpenSource() = runBlocking {
        for(name in listOf("unsupported.NEF","PDF drawing.ai")) Fixture().use { fixture ->
            fixture.displayName=name;fixture.mediaType="application/octet-stream"
            val decoder=object:WorkbenchStreamingCore {
                override suspend fun createFile(options:CreateFileProject):WorkbenchProject = error("image decoder was reached")
            }
            try {
                RasterTransfers(fixture.context).importImage(fixture.input,decoder,{_,_,_->error("project options were reached")}){}
                fail("unsupported intake reached the source")
            } catch (_:IllegalArgumentException) { }
            assertFalse(fixture.opened.isCompleted);fixture.assertNoWorkspaces()
        }
    }

    @Test fun staleNativeReceiptIsRejectedBeforeOpeningProviderWriter() = runBlocking {
        for (variant in 0..2) Fixture().use { fixture ->
            fixture.destination.createNewFile()
            val original = fixture.source.readBytes()
            val expected = ProjectInfo("expected", "synthetic", "device", 1uL, false, false, 4uL, "state", listOf("document"))
            val stale = when (variant) {
                0 -> expected.copy(projectId = "other")
                1 -> expected.copy(hostSeq = 5uL)
                else -> expected.copy(stateHash = "changed")
            }
            val exporter = object : WorkbenchStreamingProject {
                override suspend fun exportFile(options: FileExportOptions): FileExportResult {
                    File(options.outputPath).writeBytes(original)
                    return FileExportResult(original.size.toULong(), "a".repeat(64), "{}", stale, 4096uL)
                }
            }
            val failure = runCatching { RasterTransfers(fixture.context).export(fixture.output, exporter,
                ExportOptions("document"), { receipt ->
                    if (receipt.revision.projectId != expected.projectId || receipt.revision.hostSeq != expected.hostSeq ||
                        receipt.revision.stateHash != expected.stateHash) throw IllegalStateException("stale receipt")
                }) { } }.exceptionOrNull()
            assertTrue(failure is IllegalStateException)
            assertEquals(0, fixture.outputWriterOpens.get())
            assertFalse(fixture.destination.exists())
            fixture.assertNoWorkspaces(); assertArrayEquals(original, fixture.source.readBytes())
        }
    }

    @Test fun receiptCallbackRunsBeforeWriterAndCanCancelWithoutPublishing() = runBlocking {
        for (cancel in listOf(false, true)) Fixture().use { fixture ->
            fixture.destination.createNewFile()
            val original = fixture.source.readBytes()
            val revision = ProjectInfo("expected", "synthetic", "device", 1uL, false, false, 4uL, "state", listOf("document"))
            val exporter = object : WorkbenchStreamingProject {
                override suspend fun exportFile(options: FileExportOptions): FileExportResult {
                    File(options.outputPath).writeBytes(original)
                    return FileExportResult(original.size.toULong(), "a".repeat(64), "{}", revision, 4096uL)
                }
            }
            val result = runCatching { RasterTransfers(fixture.context).export(fixture.output, exporter,
                ExportOptions("document"), { receipt ->
                    assertEquals(0, fixture.outputWriterOpens.get())
                    assertEquals(revision, receipt.revision)
                    if (cancel) throw CancellationException("synthetic callback cancellation")
                }) { } }
            if (cancel) {
                assertTrue(result.exceptionOrNull() is CancellationException)
                assertEquals(0, fixture.outputWriterOpens.get()); assertFalse(fixture.destination.exists())
            } else {
                assertTrue(result.isSuccess)
                assertEquals(1, fixture.outputWriterOpens.get()); assertArrayEquals(original, fixture.destination.readBytes())
            }
            fixture.assertNoWorkspaces()
        }
    }

    @Test fun refusedShareReceiptNeverCreatesLease() = runBlocking {
        Fixture().use { fixture ->
            val original = fixture.source.readBytes()
            var leases = 0
            val exporter = object : WorkbenchStreamingProject {
                override suspend fun exportFile(options: FileExportOptions): FileExportResult {
                    File(options.outputPath).writeBytes(original)
                    return FileExportResult(original.size.toULong(), "a".repeat(64), "{}",
                        ProjectInfo("other", "synthetic", "device", 1uL, false, false, 4uL, "state", listOf("document")), 4096uL)
                }
            }
            val transfers = RasterTransfers(fixture.context) { context -> MediaHandoff(context) { _, file ->
                leases++; Uri.parse("content://synthetic.handoff/${file.name}")
            } }
            val failure = runCatching { transfers.sharePng(exporter, ExportOptions("document"),
                { receipt -> if (receipt.revision.projectId != "expected") throw IllegalStateException("stale receipt") }) { } }.exceptionOrNull()
            assertTrue(failure is IllegalStateException)
            assertEquals(0, leases)
            assertEquals(0, fixture.outputWriterOpens.get())
            fixture.assertNoWorkspaces()
        }
    }
    @Test fun cancelledExportRemovesOnlyItsNewDestinationAndWorkspace() = runBlocking {
        Fixture().use { fixture ->
            fixture.destination.writeText("new empty destination fixture")
            val ready=CompletableDeferred<Unit>()
            val exporter=object:WorkbenchStreamingProject {
                override suspend fun exportFile(options:FileExportOptions):FileExportResult {
                    File(options.outputPath).writeText("private partial output")
                    ready.complete(Unit)
                    try { awaitCancellation() } finally { File(options.outputPath).delete() }
                }
            }
            val job=launch { RasterTransfers(fixture.context).export(fixture.output,exporter,ExportOptions("fixture"),
                { _ -> fail("Cancelled exporter returned a receipt") }){ } }
            withTimeout(10_000){ready.await()};job.cancelAndJoin()
            assertFalse(fixture.destination.exists());assertTrue(fixture.source.exists());fixture.assertNoWorkspaces()
        }
    }
    @Test fun cancellationAfterNativeImportHandoffClosesReturnedHandle() = runBlocking {
        Fixture().use { fixture ->
            val core=workbenchCore();val actual=core as WorkbenchStreamingCore;val now=System.currentTimeMillis();val id=core.newId(now.toULong());val path=File(fixture.root,"project").absolutePath
            val acquired=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
            val streaming=object:WorkbenchStreamingCore {
                override suspend fun createFile(options:CreateFileProject):WorkbenchProject {
                    val handle=actual.createFile(options)
                    acquired.complete(Unit)
                    // Deliberately deliver a handle to a cancelled caller to
                    // exercise ownership during the app cleanup suspension.
                    withContext(NonCancellable){release.await()}
                    return handle
                }
            }
            val job=launch { RasterTransfers(fixture.context).importImage(fixture.input,streaming,{source,work,title->CreateFileProject(path,id,core.newId(now.toULong()),core.newId(now.toULong()),core.newDeviceId(),title,source,work,now)}){ }.close() }
            withTimeout(20_000){acquired.await()};job.cancel();release.complete(Unit);withTimeout(20_000){job.join()}
            val reopened=core.open(path);try{assertEquals(id,reopened.info().projectId)}finally{reopened.close()};fixture.assertNoWorkspaces()
        }
    }

    @Test fun typedPreflightFailureCleansScratchAndPreservesOriginal() = runBlocking {
        Fixture().use { fixture ->
            val original = fixture.source.readBytes()
            val exporter = object : WorkbenchFileTransfers {
                override suspend fun preflightFile(options: FileExportOptions): FilePreflight {
                    File(options.workDirectory, "native-partial.tmp").writeText("owned scratch")
                    throw TransferFailure(TransferFailureKind.Depth)
                }
                override suspend fun exportTransfer(options: FileExportOptions): FileExportResult = error("Preflight must not export")
                override suspend fun attachFile(options: AttachFileOptions): AttachedAsset = error("Preflight must not attach")
            }
            val failure = runCatching { RasterTransfers(fixture.context).preflight(exporter, ExportOptions("fixture", ImageFormat.Jpeg(90u))) }.exceptionOrNull()
            assertEquals(TransferFailureKind.Depth, (failure as TransferFailure).kind)
            fixture.assertNoWorkspaces(); assertArrayEquals(original, fixture.source.readBytes())
            assertFalse(fixture.destination.exists())
        }
    }

    @Test fun forgedFilesystemDestinationIsRejectedBeforeWritingOrDeletingOriginal() = runBlocking {
        Fixture().use { fixture ->
            val original = fixture.source.readBytes()
            val exporter = object : WorkbenchStreamingProject {
                override suspend fun exportFile(options: FileExportOptions): FileExportResult = error("Must reject destination before native export")
            }
            val transfers = RasterTransfers(fixture.context)
            assertTrue(runCatching { transfers.export(Uri.fromFile(fixture.source), exporter, ExportOptions("fixture"),
                { _ -> fail("Filesystem destination reached validation") }) { } }.exceptionOrNull() is IllegalArgumentException)
            assertTrue(runCatching { transfers.discardDestination(Uri.fromFile(fixture.source)) }.exceptionOrNull() is IllegalArgumentException)
            assertArrayEquals(original, fixture.source.readBytes()); fixture.assertNoWorkspaces()
        }
    }

    @Test fun cancellationAtShareLeaseHandoffRemovesOnlyOwnedPng() = runBlocking {
        Fixture().use { fixture ->
            val original = fixture.source.readBytes()
            val exporter = object : WorkbenchStreamingProject {
                override suspend fun exportFile(options: FileExportOptions): FileExportResult {
                    File(options.outputPath).writeBytes(original)
                    return FileExportResult(original.size.toULong(), "synthetic", "{}",
                        ProjectInfo("fixture", "synthetic", "device", 1uL, false, false, 0uL, "state", listOf("document")), 4096uL)
                }
            }
            lateinit var job: Job
            var leaseCreated = false
            val transfers = RasterTransfers(fixture.context) { context -> MediaHandoff(context) { _, file ->
                leaseCreated = true
                job.cancel()
                Uri.parse("content://synthetic.handoff/${file.parentFile?.name}/${file.name}")
            } }
            job = launch(start = CoroutineStart.LAZY) { transfers.sharePng(exporter, ExportOptions("document"),
                { receipt -> assertTrue(receipt.encodedBytes > 0uL) }) { } }
            job.start(); withTimeout(10_000) { job.join() }
            assertTrue(leaseCreated); assertTrue(job.isCancelled)
            fixture.assertNoWorkspaces(); assertArrayEquals(original, fixture.source.readBytes())
            assertTrue(File(fixture.root, "media-handoff/share").listFiles()?.isEmpty() == true)
        }
    }

    @Test fun successfulShareRetainsOnlyDisposablePngUntilGrantLeaseCleanup() = runBlocking {
        Fixture().use { fixture ->
            val original = fixture.source.readBytes()
            val handoff = MediaHandoff(fixture.context) { _, file -> Uri.parse("content://synthetic.handoff/${file.parentFile?.name}/${file.name}") }
            val exporter = object : WorkbenchStreamingProject {
                override suspend fun exportFile(options: FileExportOptions): FileExportResult {
                    File(options.outputPath).writeBytes(original)
                    return FileExportResult(original.size.toULong(), "synthetic", "{}",
                        ProjectInfo("fixture", "synthetic", "device", 1uL, false, false, 0uL, "state", listOf("document")), 4096uL)
                }
            }
            val (lease, receipt) = RasterTransfers(fixture.context) { handoff }.sharePng(exporter, ExportOptions("document"),
                { result -> assertTrue(result.encodedBytes > 0uL) }) { }
            try {
                fixture.assertNoWorkspaces(); assertEquals(receipt.encodedBytes, lease.file.length().toULong())
                assertArrayEquals(original, lease.file.readBytes()); assertArrayEquals(original, fixture.source.readBytes())
            } finally { handoff.remove(lease, false) }
            assertFalse(lease.file.exists())
        }
    }

    @Test fun stalledProviderReadCancelsWithoutWaitingForItsWriterAndClosesDescriptor() = runBlocking {
        Fixture().use { f ->
            f.stallInput()
            val core = object : WorkbenchStreamingCore {
                override suspend fun createFile(options: CreateFileProject): WorkbenchProject = error("Empty stalled source must not reach native import")
            }
            val job = launch { RasterTransfers(f.context).importImage(f.input, core, { _, _, _ -> error("No source is ready") }) {} }
            val opened = withTimeout(5_000) { f.opened.await() }
            withTimeout(5_000) { job.cancelAndJoin() }
            assertTrue(runCatching { opened.fd }.isFailure); f.assertNoWorkspaces(); assertTrue(f.source.exists())
        }
    }

    @Test fun stalledProviderWriteCancelsWithoutWaitingForReaderAndRemovesDestination() = runBlocking {
        Fixture().use { f ->
            f.stallOutput(); f.destination.createNewFile()
            val exporter = object : WorkbenchStreamingProject {
                override suspend fun exportFile(options: FileExportOptions): FileExportResult {
                    File(options.outputPath).writeBytes(ByteArray(512 * 1024) { 42 })
                    return FileExportResult(512uL * 1024uL, "synthetic", "{}",
                        ProjectInfo("fixture", "synthetic", "device", 1uL, false, false, 0uL, "state", listOf("document")), 4096uL)
                }
            }
            val job = launch { RasterTransfers(f.context).export(f.output, exporter, ExportOptions("document"),
                { result -> assertEquals("fixture",result.revision.projectId) }) {} }
            val opened = withTimeout(5_000) { f.opened.await() }
            delay(100) // Fill the pipe while its reader deliberately remains open and idle.
            withTimeout(5_000) { job.cancelAndJoin() }
            assertTrue(runCatching { opened.fd }.isFailure); assertFalse(f.destination.exists())
            f.assertNoWorkspaces(); assertTrue(f.source.exists())
        }
    }
}
