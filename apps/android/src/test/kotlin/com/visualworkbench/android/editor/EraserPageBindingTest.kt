package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlin.test.*

/** Actual app adapter with deterministic hit input; GPU geometry/native storage
 * are covered separately. Equal dimensions/revision must not hide a page change. */
class EraserPageBindingTest {
    @Test fun adapterRebindsSameRevisionPagesAndSettlesBeforeAnotherPage():Unit=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val core=PageCore();val project=PageProject()
        val selection=SelectionInteraction(core,scope,{1L},{})
        val owner=EraserController(core,scope,selection,{query->query.document.render.items.map{it.objectId}}){ }
        val camera=Camera(Point(32.0,32.0),1.0,0.0,64.0,64.0)
        try {
            owner.bind(project,project.snapshot(0u),1,1u,camera);owner.interaction.tool(EraserMode.Object)
            assertTrue(owner.interaction.begin(Point(10.0,10.0),camera))
            owner.bind(project,project.snapshot(1u),1,1u,camera)
            assertFalse(owner.interaction.finish());assertTrue(project.deletes.isEmpty())
            assertTrue(owner.interaction.begin(Point(10.0,10.0),camera));assertTrue(owner.interaction.finish())
            assertEquals(listOf("page-1-object"),project.deletes.single().second)
            assertEquals(1u,project.deletes.single().first.page)
            owner.detach();selection.detach()
            owner.bind(project,project.snapshot(0u),1,1u,camera);owner.interaction.tool(EraserMode.Object)
            assertTrue(owner.interaction.begin(Point(10.0,10.0),camera));assertTrue(owner.interaction.finish())
            assertEquals(listOf("page-0-object"),project.deletes.last().second);assertEquals(0u,project.deletes.last().first.page)
        }finally{owner.detach();selection.detach();scope.cancel()}
    }
}
private class PageCore:WorkbenchCore {
    override fun newId(unixMs:ULong):String="transaction"
    override fun newDeviceId():String="device"
    override fun cameraMatrix(camera:Camera,inverse:Boolean):Transform=Transform()
    override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>):List<Point> = points
    override suspend fun create(options:CreateProject):WorkbenchProject=error("unused")
    override suspend fun open(path:String):WorkbenchProject=error("unused")
    override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("unused")
}
private class PageProject:WorkbenchProject {
    val revision=ProjectInfo("project","title","device",1u,false,false,1u,"same-revision",listOf("doc"))
    val deletes=mutableListOf<Pair<EditOptions,List<String>>>()
    fun snapshot(page:UInt):DocumentSnapshot {
        val item=RenderItem("page-$page-object","page-$page-layer",1.0,"normal",Rect(0.0,0.0,20.0,20.0),
            Contours(longArrayOf(),longArrayOf(),uintArrayOf()),Transform(),ObjectStyle(0xff0000ffu,2.0),Shape.Stroke("pen"),false)
        return DocumentSnapshot("doc","title",64u,64u,8u,listOf(LayerInfo(item.layerId,"page",true,false,1.0,"normal")),RenderList(revision,listOf(item)),page)
    }
    override val changes=emptyFlow<ProjectChange>()
    override suspend fun info():ProjectInfo=revision
    override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo {
        deletes+=options to commands.map{(it as EditCommand.Delete).objectId};return revision
    }
    override suspend fun document(documentId:String):DocumentSnapshot=error("unused")
    override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("unused")
    override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("unused")
    override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("unused")
    override suspend fun render(documentId:String,rectangle:Rect?):RenderList=error("unused")
    override suspend fun export(options:ExportOptions):ExportResult=error("unused")
    override suspend fun close():Unit=Unit
}
