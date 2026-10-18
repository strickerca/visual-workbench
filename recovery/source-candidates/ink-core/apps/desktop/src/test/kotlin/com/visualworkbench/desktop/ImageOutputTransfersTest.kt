@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

class ImageOutputTransfersTest {
    @Test fun nestedFrameDimensionsDoNotReplaceCompletedTopLevelDimensions() {
        val state=state();val request=captureImageOutputExport(state,core,output())
        val text="""{"revision":"${request.revision}","source_asset":"${"b".repeat(64)}","output_width":100,"output_height":200,"image_output":{"output_width":100,"output_height":200,"crop_width":40}}"""
        val result=FileExportResult(80uL,"c".repeat(64),text,info,1024uL)
        val receipt=completeExport(request,result)
        assertEquals(100u,receipt.width);assertEquals(200u,receipt.height)
        assertTrue(state.copy(exportReceipt=receipt).exportReady)
        assertFalse(state.copy(projectEpoch=4,exportReceipt=receipt).exportReady)
    }
    @Test fun duplicateTopLevelKeysAndNestedOnlyKeysRefuse() {
        val invalid=listOf(
            """{"revision":"r1-a","source_asset":"b","output_width":2,"output_width":3,"output_height":4}""",
            """{"revision":"r1-a","source_asset":"b","image_output":{"output_width":2,"output_height":4}}""",
            """{"revision":"r1-a","source_asset":"b","output_width":2.5,"output_height":4}""",
            """{"revision":"r1-a","source_asset":"b","output_width":-2,"output_height":4}""",
            " ".repeat(65_537))
        invalid.forEach{text->assertThrows(HandoffFailure::class.java){imageOutputReceiptFields(text)}}
    }
    @Test fun missingOrStaleMetadataNeverFallsBackToOriginalDimensions() {
        assertThrows(HandoffFailure::class.java){captureImageOutputExport(state(),core,null)}
        assertThrows(HandoffFailure::class.java){captureImageOutputExport(state(),core,output().copy(binding=output().binding.copy(stateHash="f".repeat(64))))}
        val full=captureImageOutputExport(state(),core,output())
        assertEquals(100u,full.width);assertEquals(200u,full.height);assertNull(full.region)
    }
    @Test fun selectedDGeometryIsMappedIntoCommittedOutputBeforeRounding() {
        val base=state();val selected=base.copy(selected=setOf("shape"),exportSettings=base.exportSettings.copy(area=ExportArea.Selection))
        val request=captureImageOutputExport(selected,core,output())
        assertEquals(Rect(70.0,25.0,20.0,50.0),request.region)
        assertEquals(20u,request.width);assertEquals(50u,request.height)
        assertFalse(request.matches(selected.copy(selected=emptySet())))
    }
    @Test fun viewRequestRetainsCameraAndMapsOutOfCropViewToRefusal() {
        val base=state().copy(view=ViewState(Camera(Point(0.0,0.0),1.0,0.0,1.0,1.0)),exportSettings=ExportSettings(area=ExportArea.View))
        assertThrows(HandoffFailure::class.java){captureImageOutputExport(base,core,output())}
        val large=base.copy(view=base.view.copy(camera=base.view.camera.copy(viewportWidth=100.0,viewportHeight=80.0)))
        val request=captureImageOutputExport(large,core,output())
        assertEquals(Rect(0.0,0.0,100.0,200.0),request.region)
        assertFalse(request.matches(large.copy(view=large.view.copy(camera=large.view.camera.copy(rotation=.2)))))
    }
    private fun output()=ImageOutputSnapshot(WorkflowBinding(info.projectId,"document",info.hostSeq,info.stateHash),"b".repeat(64),100u,80u,8u,
        ImageOutputEntry("output-layer","output-object",true,false,ImageOutputSettings(10u,20u,40u,20u,90u,100u,200u)),emptyList())
    private fun state():EditorState {
        val item=RenderItem("shape","layer",1.0,"normal",Rect(15.0,22.0,10.0,4.0),Contours(longArrayOf(),longArrayOf(),uintArrayOf()),
            Transform(),ObjectStyle(0xffffffffu,1.0),Shape.Rectangle(Rect(15.0,22.0,10.0,4.0)),false)
        return EditorState(document=DocumentSnapshot("document","Source",100u,80u,8u,emptyList(),RenderList(info,listOf(item))),projectEpoch=3)
    }
    private val info=ProjectInfo("project","Source","device",2uL,true,false,1uL,"a".repeat(64),listOf("document"))
    /** The view fixture is already D-aligned; no replacement geometry math. */
    private val core=object:WorkbenchCore {
        override fun newId(unixMs:ULong):String=error("no IDs")
        override fun newDeviceId():String=error("no device")
        override suspend fun create(options:CreateProject):WorkbenchProject=error("no project")
        override suspend fun open(path:String):WorkbenchProject=error("no file")
        override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("no text")
        override fun cameraMatrix(camera:Camera,inverse:Boolean)=Transform()
        override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>)=points
    }
}
