package com.visualworkbench.android

import android.content.ContentProvider
import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.ProviderInfo
import android.database.Cursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.MediaHandoff
import com.visualworkbench.android.editor.RasterTransfers
import com.visualworkbench.shared.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Real transfer boundary, synthetic private provider and controlled native
 * receipt. No real provider, app grant, clipboard or network is touched. */
@RunWith(AndroidJUnit4::class)
class ImageOutputPublicationInstrumentedTest {
    private val before=ProjectInfo("project","Synthetic","device",9uL,true,false,4uL,"a".repeat(64),listOf("document"))
    private val binding=WorkflowBinding(before.projectId,"document",before.hostSeq,before.stateHash)
    private val changed=before.copy(hostSeq=5uL,stateHash="b".repeat(64))
    private val exporter=object:WorkbenchStreamingProject {
        override suspend fun exportFile(options:FileExportOptions):FileExportResult {
            File(options.outputPath).writeBytes(byteArrayOf(1,2,3,4))
            return FileExportResult(4uL,"c".repeat(64),"{}",changed,1024uL)
        }
    }

    @Test fun changed_revision_never_opens_or_writes_the_selected_destination()=runBlocking {
        Fixture().use { fixture ->
            fixture.outputFile.writeBytes(byteArrayOf()) // CreateDocument owns this new empty URI.
            val error=runCatching { RasterTransfers(fixture.context).exportBound(fixture.output,exporter,ExportOptions("document"),binding){} }.exceptionOrNull()
            assertTrue(error is IllegalArgumentException)
            assertEquals(0,fixture.opens);assertFalse(fixture.outputFile.exists())
            fixture.assertNoPrivateOutputs()
        }
    }
    @Test fun changed_revision_never_creates_a_share_lease_or_uri()=runBlocking {
        Fixture().use { fixture ->
            var uriCreated=0
            val transfers=RasterTransfers(fixture.context){ctx->MediaHandoff(ctx){_,file->uriCreated++;Uri.parse("content://synthetic.share/${file.name}")}}
            val error=runCatching { transfers.sharePngBound(exporter,ExportOptions("document"),binding){} }.exceptionOrNull()
            assertTrue(error is IllegalArgumentException);assertEquals(0,uriCreated);assertEquals(0,fixture.opens)
            assertTrue(File(fixture.root,"media-handoff/share").listFiles().isNullOrEmpty())
            fixture.assertNoPrivateOutputs()
        }
    }
    private class Fixture:AutoCloseable {
        private val actual=InstrumentationRegistry.getInstrumentation().targetContext
        val root=File(actual.cacheDir.canonicalFile,"output-publication-${UUID.randomUUID()}").also{check(it.mkdir())}
        val outputFile=File(root,"destination.png")
        val output:Uri=Uri.parse("content://synthetic.output.fixture/export")
        var opens=0
        private lateinit var resolver:ContentResolver
        val context=object:ContextWrapper(actual) {
            override fun getApplicationContext():Context=this
            override fun getCacheDir():File=root
            override fun getContentResolver():ContentResolver=resolver
        }
        init {
            val provider=object:ContentProvider() {
                override fun onCreate():Boolean=true
                override fun query(uri:Uri,projection:Array<out String>?,selection:String?,selectionArgs:Array<out String>?,sortOrder:String?):Cursor?=null
                override fun getType(uri:Uri):String="image/png"
                override fun insert(uri:Uri,values:ContentValues?):Uri?=null
                override fun update(uri:Uri,values:ContentValues?,selection:String?,selectionArgs:Array<out String>?):Int=0
                override fun delete(uri:Uri,selection:String?,selectionArgs:Array<out String>?):Int=if(uri==output&&outputFile.delete())1 else 0
                override fun openFile(uri:Uri,mode:String):ParcelFileDescriptor { opens++;return ParcelFileDescriptor.open(outputFile,ParcelFileDescriptor.parseMode(mode)) }
            }
            provider.attachInfo(context,ProviderInfo().apply{authority="synthetic.output.fixture";exported=false})
            resolver=ContentResolver.wrap(provider)
        }
        fun assertNoPrivateOutputs(){assertTrue(File(root,"raster-transfers").listFiles().isNullOrEmpty())}
        override fun close(){assertEquals(actual.cacheDir.canonicalFile,root.canonicalFile.parentFile);check(root.name.startsWith("output-publication-"));assertTrue(root.deleteRecursively())}
    }
}
