package com.visualworkbench.shared

import kotlin.test.*

class GiantViewGeometryTest {
    private val camera = Camera(Point(128.0,128.0),1.0,0.0,256.0,256.0)
    private val core = object : WorkbenchCore {
        override fun newId(unixMs:ULong):String=error("Unused")
        override fun newDeviceId():String=error("Unused")
        override suspend fun create(options:CreateProject):WorkbenchProject=error("Unused")
        override suspend fun open(path:String):WorkbenchProject=error("Unused")
        override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("Unused")
        override fun cameraMatrix(camera:Camera,inverse:Boolean):Transform=Transform()
        override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>):List<Point> = points
    }
    @Test fun precisionFootprintAndPhysicalScaleAreIncludedWithoutAnotherDpiFactor() {
        val pane=MagnifierPane(Rect(10.0,10.0,20.0,20.0),Transform(a=2.0,d=2.0,e=-600.0),2.0,2,false,true)
        val view=giantViewport(core,camera,PrecisionFrame(lens=pane))
        assertEquals(0.0,view.left);assertEquals(315.0,view.right)
        assertEquals(0.0,view.top);assertEquals(256.0,view.bottom)
        assertEquals(2.0,view.a);assertEquals(2.0,view.d)
    }
    @Test fun singularPaneAndNonfiniteViewportRefuseBeforeNativePlanning() {
        val pane=MagnifierPane(Rect(0.0,0.0,20.0,20.0),Transform(a=1.0,b=2.0,c=2.0,d=4.0),2.0,2,false,true)
        assertFailsWith<IllegalArgumentException>{giantViewport(core,camera,PrecisionFrame(lens=pane))}
        assertFailsWith<IllegalArgumentException>{giantViewport(core,camera.copy(viewportWidth=Double.NaN))}
    }
    @Test fun drawFenceIncludesFullRevisionAttachmentAndExactViewEnvelope() {
        val binding=WorkflowBinding("project","document",1u,"a".repeat(64));val view=giantViewport(core,camera)
        val frame=GiantVisibleFrame<Int>(binding,3,1u,view,"b".repeat(64),"c".repeat(64),emptyList())
        assertTrue(frame.matches(binding,3,view))
        assertFalse(frame.matches(binding.copy(stateHash="d".repeat(64)),3,view))
        assertFalse(frame.matches(binding,4,view))
        assertFalse(frame.matches(binding,3,view.copy(left=1.0)))
    }
}
