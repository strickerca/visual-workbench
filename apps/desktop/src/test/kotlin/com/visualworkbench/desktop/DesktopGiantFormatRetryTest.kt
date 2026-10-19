package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.Executors

/** A failed format read cannot require a revision change to unlock Retry. */
class DesktopGiantFormatRetryTest {
    private class Project:WorkbenchProject,WorkbenchOriginalFormats,WorkbenchGiantImagesProject {
        val document=DocumentSnapshot("document","Giant",100_000u,100_000u,8u,emptyList(),RenderList(
            ProjectInfo("project","Giant","owner",8u,true,false,5u,"c".repeat(64),listOf("document")),emptyList()))
        var formatReads=0
        var giantOpens=0
        var closes=0
        override val changes=MutableSharedFlow<ProjectChange>()
        override suspend fun originalFormatInfo(binding:WorkflowBinding):OriginalFormatInfo {
            formatReads++
            if(formatReads==1)error("transient format failure")
            return OriginalFormatInfo(binding,"a".repeat(64),"png",8u,false,false)
        }
        override suspend fun openGiantImage(binding:WorkflowBinding,diskCacheBytes:ULong,ramCacheBytes:ULong):GiantImage {
            giantOpens++
            throw GiantImageFailure(GiantImageFailureKind.Unsupported)
        }
        override suspend fun info()=document.render.revision
        override suspend fun document(documentId:String)=document
        override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("full bitmap forbidden")
        override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("Unused")
        override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("Unused")
        override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("Unused")
        override suspend fun render(documentId:String,rectangle:Rect?)=document.render
        override suspend fun export(options:ExportOptions):ExportResult=error("Unused")
        override suspend fun close(){closes++}
    }
    private class Core(private val project:Project):WorkbenchCore {
        override fun newId(unixMs:ULong)="giant-format-test"
        override fun newDeviceId()=error("Unused")
        override suspend fun create(options:CreateProject):WorkbenchProject=error("Unused")
        override suspend fun open(path:String):WorkbenchProject=project
        override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("Unused")
        override fun cameraMatrix(camera:Camera,inverse:Boolean)=Transform()
        override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>)=points
    }

    @Test fun retryRefreshesMissingFormatReceiptAndAttemptsGiantOpenAtTheSameRevision(){
        val root=Files.createTempDirectory("vw-giant-format-retry-")
        val project=Project()
        try {
            Executors.newSingleThreadExecutor{task->Thread(task,"vw-giant-format-retry").apply{isDaemon=true}}
                .asCoroutineDispatcher().use{dispatcher->runBlocking(dispatcher){
                    val lifetime=SupervisorJob(coroutineContext[Job])
                    val editor=EditorController(CoroutineScope(coroutineContext+lifetime),Core(project),object:DesktopHandoff{
                        override suspend fun paste(assumeSrgb:Boolean):ClipboardImage=error("Unused")
                        override suspend fun copy(path:Path,receipt:CompletedExport):Unit=error("Unused")
                        override suspend fun stage(path:Path,receipt:CompletedExport):DesktopDragFile=error("Unused")
                    },LocationPolicy(root.resolve("projects"),false,"isolated test"),"owner",root.resolve("private"))
                    suspend fun idle(ready:(EditorState)->Boolean):EditorState=withTimeout(5000){
                        combine(editor.state,editor.instructionEditor.state,editor.semanticEditor.state){state,instructions,semantics->
                            state.takeIf{!it.busy&&!instructions.busy&&!semantics.busy&&ready(it)}
                        }.first{it!=null}!!
                    }
                    try {
                        editor.open(root.resolve("project.sqlite"))
                        val failed=idle{it.originalFormatUnavailable&&it.backgroundIssue!=null}
                        val revision=checkNotNull(failed.document).render.revision
                        assertNull(failed.originalFormatInfo)
                        assertEquals(1,project.formatReads)
                        assertEquals(0,project.giantOpens)
                        checkNotNull(editor.retryBackground()).join()
                        val recovered=idle{it.originalFormatInfo!=null&&it.giantIssue!=null}
                        assertEquals(revision,recovered.document?.render?.revision)
                        assertEquals(2,project.formatReads)
                        assertEquals(1,project.giantOpens)
                        assertEquals("a".repeat(64),recovered.originalFormatInfo?.sourceAssetId)
                        assertEquals(0,project.closes)
                    } finally {withContext(NonCancellable){lifetime.cancelAndJoin();editor.close()}}
                }}
        } finally {
            Files.walk(root).use{stream->stream.sorted(java.util.Comparator.reverseOrder<Path>()).forEach{Files.deleteIfExists(it)}}
        }
        assertEquals(1,project.closes)
    }
}
