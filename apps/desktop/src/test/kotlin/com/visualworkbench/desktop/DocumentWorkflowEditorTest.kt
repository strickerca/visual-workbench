package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.util.Comparator
import java.util.concurrent.Executors
import javax.imageio.ImageIO

/** Real borrowed project, 24-document chooser and device-local saved camera.
 * Synthetic headless controller calls do not establish Window/device UX. */
class DocumentWorkflowEditorTest {
    @Test fun twentyFourDocumentsSwitchAndSavedViewSurvivesReturnAndReopen(){
        Executors.newSingleThreadExecutor().asCoroutineDispatcher().use{dispatcher->runBlocking(dispatcher){
            val root=Files.createTempDirectory("vw-doc-workflow-");val scope=CoroutineScope(coroutineContext+SupervisorJob())
            val core=workbenchCore();val now=System.currentTimeMillis();val device=core.newDeviceId()
            fun id()=core.newId(now.toULong())
            val path=root.resolve("project");val first=id();var editor:EditorController?=null
            fun createEditor()=EditorController(scope,core,object:DesktopHandoff {
                override suspend fun paste(assumeSrgb:Boolean):ClipboardImage=error("unused")
                override suspend fun copy(path:java.nio.file.Path,receipt:CompletedExport)=error("unused")
                override suspend fun stage(path:java.nio.file.Path,receipt:CompletedExport):DesktopDragFile=error("unused")
            },LocationPolicy(root,true,"owned synthetic fixture"),device)
            try{
                val bytes=ByteArrayOutputStream().also{check(ImageIO.write(BufferedImage(64,64,BufferedImage.TYPE_INT_RGB),"png",it))}.toByteArray()
                val project=core.create(CreateProject(path.toString(),id(),first,id(),device,"Document 1",bytes,now))
                try{
                    val controls=documentControlWorkflows(project)
                    for(index in 2..24){
                        val info=project.info();val binding=WorkflowBinding(info.projectId,first,info.hostSeq,info.stateHash)
                        val plan=controls.prepare(binding,WorkflowMetadata(id(),device,info.nextLamport,now),
                            DocumentControlCommand.CreateDocumentFromSource(id(),id(),"Document $index"))
                        try{plan.commit()}finally{plan.close()}
                    }
                }finally{project.close()}
                val owner=createEditor();editor=owner;owner.viewport(200.0,200.0);owner.open(path);ready(owner)
                val original=checkNotNull(owner.state.value.document).documentId
                assertEquals(24,owner.workflows.navigator.state.value.documentIds.size)
                owner.zoom(2.0);owner.pan(12.0,8.0);val saved=owner.state.value.view.camera
                owner.workflows.activate(EditorWorkflowPanel.SavedViews,true)
                val views=owner.workflows.views.state.value
                owner.workflows.saveView(checkNotNull(views.binding),checkNotNull(views.attachmentEpoch),"Detail")
                until{owner.workflows.views.state.value.bookmarks.size==1&&!owner.workflows.views.state.value.saving}
                owner.workflows.activate(EditorWorkflowPanel.SavedViews,false)
                suspend fun choose(target:String){
                    owner.workflows.activate(EditorWorkflowPanel.Documents,true)
                    val catalog=owner.workflows.navigator.state.value
                    val request=checkNotNull(owner.workflows.navigator.request(checkNotNull(catalog.binding),checkNotNull(catalog.attachmentEpoch),target)){"panel=${owner.workflows.panel.value}; busy=${owner.state.value.busy}; selection=${owner.selections.state.value.busy}; eraser=${owner.erasers.state.value.busy}; review=${owner.workflows.review.state.value.busy}; issue=${owner.state.value.message}"}
                    owner.workflows.choose(request)
                    until{owner.state.value.document?.documentId==target&&owner.workflows.panel.value==EditorWorkflowPanel.None&&!owner.workflows.blocksInput()}
                    ready(owner)
                }
                choose(owner.workflows.navigator.state.value.documentIds.first{it!=original})
                choose(original);assertEquals(saved,owner.state.value.view.camera)
                owner.close();editor=null
                val reopened=createEditor();editor=reopened;reopened.viewport(200.0,200.0);reopened.open(path);ready(reopened)
                assertEquals(original,reopened.state.value.document?.documentId)
                assertEquals(saved,reopened.state.value.view.camera)
                assertEquals(listOf("Detail"),reopened.workflows.views.state.value.bookmarks.map{it.name})
            }finally{
                editor?.close();scope.coroutineContext[Job]?.cancelAndJoin()
                Files.walk(root).use{paths->paths.sorted(Comparator.reverseOrder()).forEach{Files.delete(it)}}
            }
        }}
    }
    private suspend fun ready(editor:EditorController){
        until{!editor.state.value.busy&&(editor.state.value.background!=null||editor.state.value.needsColorConsent)}
        if(editor.state.value.needsColorConsent)editor.assumeSrgb()
        until{!editor.state.value.busy&&editor.state.value.background!=null&&!editor.workflows.blocksInput()&&!editor.instructionEditor.state.value.busy&&!editor.semanticEditor.state.value.busy&&!editor.selections.state.value.busy&&!editor.erasers.state.value.busy}
    }
    private suspend fun until(test:()->Boolean){withTimeout(15_000){while(!test())delay(5)}}
}
