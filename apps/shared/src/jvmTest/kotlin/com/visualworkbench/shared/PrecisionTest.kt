package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

/** These use the real generated geometry boundary. They are source tests until
 * the parent compiles both native targets and runs its bound shared runner. */
class PrecisionTest {
    private val camera=Camera(Point(200.0,150.0),1.25,.43,1000.0,800.0)
    private fun fixture(block:(WorkbenchCore,PrecisionController)->Unit) {
        val core=workbenchCore();val controller=PrecisionController(core,{})
        try{controller.synchronize(camera);block(core,controller)}finally{controller.close()}
    }
    @Test fun inspectionNeverChangesAcceptedTraceBits()=fixture{_,p->
        val trace=(0..199).map{Point(it*2.125-0.0,100.0+it*.125)}
        p.inspection(false);p.down(trace.first());val off=p.route(trace);p.end()
        p.inspection(true);p.down(trace.first());val on=p.route(trace);p.end()
        trace.indices.forEach{assertEquals(off[it].x.toBits(),on[it].x.toBits());assertEquals(off[it].y.toBits(),on[it].y.toBits())}
    }
    @Test fun clickQueuesUntilUpAndClosePreservesExactCameraObject()=fixture{_,p->
        val base=p.baseCamera();p.down(Point(10.0,20.0));p.barrel(true,100);p.barrel(false,200)
        assertTrue(p.state.value.pending);assertNull(p.frame().lens)
        p.end();assertNotNull(p.frame().lens);p.toggle();assertNull(p.frame().lens);assertSame(base,p.baseCamera())
        assertEquals(camera.scale.toBits(),p.baseCamera()!!.scale.toBits());assertEquals(camera.rotation.toBits(),p.baseCamera()!!.rotation.toBits())
    }
    @Test fun heldBarrelIsPeekOnlyAndNoHoverCapabilityNeedsNoSyntheticPoint()=fixture{_,p->
        p.inspection(false);p.hover(Point(700.0,600.0));assertNull(p.frame().loupe)
        p.barrel(true,0);assertNotNull(p.frame().loupe);p.barrel(false,251);assertNull(p.frame().lens);assertNull(p.frame().loupe)
        p.inspection(true);p.hover(null);assertNull(p.frame().loupe);assertNull(p.frame().reticle)
    }
    @Test fun insetSamplesStayLatchedAcrossMovedResizedToggledRequests()=fixture{core,p->
        p.hover(Point(300.0,250.0));p.toggle();val pane=p.frame().lens!!;val center=Point(pane.rectangle.x+pane.rectangle.width/2,pane.rectangle.y+pane.rectangle.height/2)
        p.down(center);val before=p.route(listOf(center)).single();p.moveLens(80.0,40.0);p.size(.55);p.magnification(8)
        assertTrue(p.state.value.pending);assertEquals(pane,p.frame().lens)
        assertEquals(before,p.route(listOf(center)).single());p.end();assertEquals(8,p.frame().lens!!.magnification)
        val doc=core.mapPoints(camera,true,listOf(before)).single();val target=core.mapPoints(camera,true,listOf(Point(300.0,250.0))).single()
        assertEquals(target.x,doc.x,1e-6);assertEquals(target.y,doc.y,1e-6)
    }
    @Test fun outsideDownNeverSwitchesMappingWhenItCrossesTheInset()=fixture{_,p->
        p.toggle();val center=Point(500.0,400.0);p.down(Point(0.0,0.0));p.move(center)
        assertEquals(center,p.route(listOf(center)).single());assertNotNull(p.frame().lens);assertEquals(4,p.frame().overview.size);p.end()
    }
    @Test fun predictionsUseSameMappingButCannotMoveOrToggleInputState()=fixture{_,p->
        p.toggle();p.down(Point(500.0,400.0));val before=p.frame();val values=listOf(Point(506.0,407.0),Point(512.0,414.0))
        assertEquals(p.route(values),p.route(values));assertEquals(before,p.frame());assertTrue(p.state.value.latched);p.cancel();assertFalse(p.state.value.latched)
    }
    @Test fun resetDropsQueuedChangesAndProjectViewCannotChangeMidstroke()=fixture{_,p->
        p.down(Point(1.0,2.0));p.toggle();try{p.synchronize(camera.copy(scale=2.0));fail("midstroke base changed")}catch(_:IllegalStateException){}
        assertSame(camera,p.baseCamera());p.reset();assertFalse(p.state.value.pending);assertNull(p.frame().lens)
        val next=camera.copy(scale=2.0);p.synchronize(next);assertSame(next,p.baseCamera())
    }
    @Test fun allViewportEdgesAvoidTheNibAndRemainContained(){
        for(left in listOf(false,true))for(x in listOf(0.0,1.0,300.0,999.0))for(y in listOf(0.0,1.0,400.0,799.0)){
            val nib=Point(x,y);val r=inspectionRectangle(camera,nib,250.0,left)!!
            assertFalse(r.contains(nib));assertTrue(r.x>=0&&r.y>=0&&r.x+r.width<=1000&&r.y+r.height<=800)
        }
        val small=Camera(Point(0.0,0.0),1.0,0.0,20.0,20.0)
        assertNull(inspectionRectangle(small,Point(10.0,10.0),20.0,false))
    }
    @Test fun mirroredDefaultAndUnsafeDraggedPlacementHaveDeterministicFallback(){
        val nib=Point(500.0,400.0);val right=inspectionRectangle(camera,nib,100.0,false)!!;val left=inspectionRectangle(camera,nib,100.0,true)!!
        assertTrue(right.x<nib.x&&right.y<nib.y);assertTrue(left.x>nib.x&&left.y<nib.y)
        assertFalse(inspectionRectangle(camera,nib,100.0,false,Point(490.0,390.0))!!.contains(nib))
    }
    @Test fun pixelGridIsRotatedDocumentEdgesAndHasBoundedWork(){
        val pane=MagnifierPane(Rect(0.0,0.0,300.0,300.0),Transform(a=0.0,b=16.0,c=-16.0,d=0.0,e=160.0,f=160.0),16.0,4,false,true)
        val grid=precisionPixelGrid(pane);assertTrue(grid.isNotEmpty());assertTrue(grid.size<=260)
        assertTrue(grid.all{(a,b)->a.x==b.x||a.y==b.y})
        assertTrue(precisionPixelGrid(pane.copy(matrix=Transform(a=.0001,d=.0001))).isEmpty())
        assertTrue(precisionPixelGrid(pane.copy(matrix=Transform(a=Double.NaN))).isEmpty())
    }
    @Test fun malformedBatchAndMagnificationLeaveTheOpenViewUntouched()=fixture{_,p->
        p.toggle();val before=p.frame().lens
        try{p.magnification(3);fail("invalid magnification")}catch(_:IllegalArgumentException){}
        try{p.route(List(513){Point(0.0,0.0)});fail("oversized batch")}catch(_:IllegalArgumentException){}
        try{p.route(listOf(Point(Double.NaN,0.0)));fail("non-finite sample")}catch(_:IllegalArgumentException){}
        try{p.inputMatrix(Point(Double.POSITIVE_INFINITY,0.0));fail("non-finite contact")}catch(_:IllegalArgumentException){}
        assertEquals(before,p.frame().lens)
    }
    @Test fun physicalSnapMatrixFollowsTheDownChoiceAndSurvivesQueuedClose()=fixture{_,p->
        p.toggle();val base=p.inputMatrix();p.down(Point(500.0,400.0));val physical=p.inputMatrix()
        assertEquals(base.a*4,physical.a,1e-12);assertEquals(base.b*4,physical.b,1e-12)
        p.toggle();assertEquals(physical,p.inputMatrix());p.end();assertEquals(base,p.inputMatrix())
        p.toggle();p.down(Point(0.0,0.0));assertEquals(base,p.inputMatrix());p.end()
    }
    @Test fun queuedCloseThenReopenKeepsTheLatchedDocumentFocus()=fixture{core,p->
        p.hover(Point(230.0,190.0));p.toggle()
        val rect=p.frame().lens!!.rectangle
        val nib=Point(rect.x+rect.width*.7,rect.y+rect.height*.6)
        p.down(nib)
        val expected=core.mapPoints(camera,true,p.route(listOf(nib))).single()
        p.toggle();p.toggle();assertTrue(p.state.value.pending);p.end()
        val matrix=p.frame().lens!!.matrix
        val mapped=Point(matrix.a*expected.x+matrix.c*expected.y+matrix.e,matrix.b*expected.x+matrix.d*expected.y+matrix.f)
        val reopened=p.frame().lens!!.rectangle
        assertEquals(reopened.x+reopened.width/2,mapped.x,1e-6)
        assertEquals(reopened.y+reopened.height/2,mapped.y,1e-6)
    }
    @Test fun refusedNativeChangesPreservePreferencesAndQueuedClose(){
        val core=workbenchCore();var reject=false
        val p=PrecisionController(core,{},factory={ value->
            val native=precisionGeometry(value)
            object:WorkbenchPrecision by native {
                override fun configure(focus:Point,center:Point,magnification:Int){
                    if(reject)throw CoreFailure(CoreFailureKind.Backpressure)
                    native.configure(focus,center,magnification)
                }
            }
        })
        try {
            p.synchronize(camera);p.toggle();p.down(Point(500.0,400.0))
            val before=p.state.value;reject=true
            try{p.magnification(8);fail("native refusal ignored")}catch(_:CoreFailure){}
            assertEquals(before,p.state.value)
            try{p.size(.55);fail("native refusal ignored")}catch(_:CoreFailure){}
            assertEquals(before,p.state.value)
            p.toggle();val pending=p.state.value
            try{p.toggle();fail("native refusal replaced close")}catch(_:CoreFailure){}
            assertEquals(pending,p.state.value);p.cancel();assertNull(p.frame().lens)
        }finally{p.close()}
    }
    @Test fun disposedControllerRefusesInputAndDoesNotNotifyAgain(){
        var changed=0
        val p=PrecisionController(workbenchCore(),{changed++})
        p.synchronize(camera);p.toggle();p.close();val notifications=changed
        p.close();assertEquals(notifications,changed);assertEquals(PrecisionState(),p.state.value)
        val commands:List<()->Unit> = listOf(
            {p.hover(Point(1.0,2.0))},{p.down(Point(1.0,2.0))},{p.route(emptyList())},
            {p.barrel(true,1)},{p.toggle()},{p.inspection(false)},{p.smooth(true)},
            {p.leftHanded(true)},{p.size(.5)},{p.magnification(8)},
            {p.moveLens(1.0,1.0)},{p.moveLoupe(1.0,1.0)},{p.resetLoupePosition()},
            {p.move(Point(1.0,2.0))},{p.end()},{p.reset()},{p.synchronize(camera)},{p.inputMatrix()})
        commands.forEach{command->try{command();fail("disposed controller admitted command")}catch(_:IllegalStateException){}}
        assertEquals(notifications,changed);assertEquals(PrecisionState(),p.state.value)
    }
    @Test fun hoverToContactDoesNotRestartTheBarrelHoldClock()=fixture{_,p->
        p.inspection(false);p.hover(Point(700.0,600.0));p.barrel(true,100)
        p.down(Point(700.0,600.0));p.barrel(true,320);p.barrel(false,351)
        assertNull(p.frame().lens);assertNull(p.frame().loupe);assertFalse(p.state.value.pending)
        p.end()
    }
    @Test fun cancellingAQueuedCloseRestoresTheBaseAndClearsAim()=fixture{_,p->
        p.toggle();val base=p.inputMatrix();p.down(Point(500.0,400.0))
        p.toggle();assertTrue(p.state.value.pending);p.cancel()
        assertEquals(base,p.inputMatrix());assertNull(p.frame().lens);assertNull(p.frame().reticle)
        assertFalse(p.state.value.pending);assertFalse(p.state.value.latched)
        val tiny=camera.copy(viewportWidth=.5,viewportHeight=.75)
        p.synchronize(tiny);p.toggle();val rect=p.frame().lens!!.rectangle
        assertTrue(rect.width>0&&rect.height>0&&rect.x+rect.width<=.5&&rect.y+rect.height<=.75)
    }

}
